use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Future, FutureExt, StreamExt};
use nimiq_block::Block;
use nimiq_database::Environment;
use nimiq_genesis::NetworkInfo;
use nimiq_nano_primitives::state_commitment;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_network_interface::{network::Network, request::request_handler};
use nimiq_utils::observer::NotifierStream;
use tokio::sync::oneshot::{self, Receiver};

use pin_project::pin_project;
use tokio::task::JoinHandle;

use crate::proof_component::{self as ZKProofComponent, ProofStore};
use crate::types::*;
use crate::zkp_requests::ZKPRequests;
use futures::stream::BoxStream;

pub type ZKProofsStream<N> = BoxStream<'static, (ZKProof, <N as Network>::PubsubId)>;

pub struct ZKPComponentProxy<N: Network> {
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    genesis_state: Vec<u8>,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
}

impl<N: Network> Clone for ZKPComponentProxy<N> {
    fn clone(&self) -> Self {
        Self {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            genesis_state: self.genesis_state.clone(),
            zkp_requests: Arc::clone(&self.zkp_requests),
        }
    }
}

impl<N: Network> ZKPComponentProxy<N> {
    pub fn get_zkp_state(&self) -> ZKPState {
        self.zkp_state.read().clone()
    }

    pub fn request_zkp_from_peers(&mut self, peers: Vec<N::PeerId>) -> bool {
        let mut zkp_requests_l = self.zkp_requests.lock();
        if zkp_requests_l.is_finished() {
            zkp_requests_l.request_zkps(peers, self.zkp_state.read().latest_block_number);
            return true;
        }
        false
    }
}

/// Election blocks ZKP Component. Has:
///
/// - The previous election block public keys
/// - Previous election block header hash
/// - The pk tree root at genesis or none if genesis block
///
/// The proofs are returned by polling the components.
#[pin_project]
pub struct ZKPComponent<N: Network> {
    blockchain: Arc<RwLock<Blockchain>>,
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    genesis_state: Vec<u8>,
    receiver: Option<Receiver<Result<ZKPState, ZKPComponentError>>>,
    blockchain_election_rx: Option<NotifierStream<BlockchainEvent>>,
    zk_proofs_stream: ZKProofsStream<N>,
    proof_generation_thread: Option<JoinHandle<()>>,
    proof_storage: ProofStore,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
}

impl<N: Network> ZKPComponent<N> {
    pub async fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        is_prover_active: bool,
        env: Environment,
    ) -> Self {
        // Defaults zkp state to genesis
        let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
        let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();

        let mut zkp_state = Arc::new(RwLock::new(
            ZKPState::with_genesis(&genesis_block).expect("Invalid genesis block"),
        ));
        let public_keys = zkp_state.read().latest_pks.clone();
        let genesis_state = state_commitment(
            genesis_block.block_number(),
            genesis_block.hash().into(),
            public_keys,
        );

        // Load from db the preexisting proof if any
        let proof_storage = ProofStore::new(env);
        if let Some(loaded_zkp_state) = proof_storage.get_zkp() {
            //let loaded_zkp_state = loaded_zkp_state;
            let proof = loaded_zkp_state.clone().into();
            let loaded_zkp_state = Arc::new(RwLock::new(loaded_zkp_state));
            if let Err(e) = Self::do_push_proof(
                &blockchain,
                &loaded_zkp_state,
                proof,
                &mut None,
                &mut None,
                &proof_storage,
                false,
            ) {
                log::error!("Error pushing the zk proof load from disk {}", e);
                proof_storage.set_zkp(&zkp_state.read());
            } else {
                log::info!("The zk proof was successfully load from disk");
                zkp_state = loaded_zkp_state;
            }
        } else {
            log::trace!("No zk proof found on the db, starts from genesis");
        }

        // If we are an active prover, the component will listen to the election block events
        let mut blockchain_election_rx = None;
        if is_prover_active {
            blockchain_election_rx = Some(blockchain.write().notifier.as_stream());
        }

        // Sets the stream of the broadcasted
        let zk_proofs_stream = network.subscribe::<ZKProofTopic>().await.unwrap().boxed();

        let zkp_component = Self {
            blockchain,
            network: Arc::clone(&network),
            zkp_state,
            genesis_state,
            receiver: None,
            blockchain_election_rx,
            zk_proofs_stream,
            proof_generation_thread: None,
            proof_storage,
            zkp_requests: Arc::new(Mutex::new(ZKPRequests::new(network))),
        };
        zkp_component.launch_request_handler();
        zkp_component
    }

    fn launch_request_handler(&self) {
        let stream = self.network.receive_requests::<RequestZKP>();
        tokio::spawn(request_handler(&self.network, stream, &self.zkp_state));
    }

    pub fn proxy(&self) -> ZKPComponentProxy<N> {
        ZKPComponentProxy {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            genesis_state: self.genesis_state.clone(),
            zkp_requests: Arc::clone(&self.zkp_requests),
        }
    }

    pub fn broadcast_zk_proof(network: &Arc<N>, zk_proof: ZKProof) {
        let network = Arc::clone(network);
        tokio::spawn(async move {
            if let Err(e) = network.publish::<ZKProofTopic>(zk_proof).await {
                log::warn!(error = &e as &dyn Error, "Failed to publish the zk proof");
            }
        });
    }

    pub fn is_zkp_prover_activated(&self) -> bool {
        self.blockchain_election_rx.is_some()
    }

    pub fn push_proof(&mut self, proof: ZKProof) -> Result<(), ZKPComponentError> {
        Self::do_push_proof(
            &self.blockchain,
            &self.zkp_state,
            proof,
            &mut self.receiver,
            &mut self.proof_generation_thread,
            &self.proof_storage,
            true,
        )
    }

    fn do_push_proof(
        blockchain: &Arc<RwLock<Blockchain>>,
        zkp_state: &Arc<RwLock<ZKPState>>,
        proof: ZKProof,
        receiver: &mut Option<Receiver<Result<ZKPState, ZKPComponentError>>>,
        proof_generation_thread: &mut Option<JoinHandle<()>>,
        proof_storage: &ProofStore,
        add_to_storage: bool,
    ) -> Result<(), ZKPComponentError> {
        let (new_block, genesis_block, proof) =
            ZKProofComponent::pre_proof_validity_checks(blockchain, proof)?;

        let zkp_state_lock = zkp_state.upgradable_read();
        if new_block.block_number() <= zkp_state_lock.latest_block_number {
            return Err(ZKPComponentError::OutdatedProof);
        }

        if let Ok(Some(new_zkp_state)) =
            ZKProofComponent::validate_proof_get_new_state(proof, new_block, genesis_block)
        {
            let mut zkp_state_lock = RwLockUpgradableReadGuard::upgrade(zkp_state_lock);
            *zkp_state_lock = new_zkp_state;

            // Since someone else generate a valide proof faster, we will terminate our own proof generation.
            *receiver = None;
            if let Some(ref thread) = proof_generation_thread {
                thread.abort();
                *proof_generation_thread = None;
            }
            if add_to_storage {
                proof_storage.set_zkp(&zkp_state_lock)
            }
            return Ok(());
        }
        Err(ZKPComponentError::InvalidProof)
    }
}

impl<N: Network> Future for ZKPComponent<N> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();

        // Check if we have requested and are waiting for peers proofs
        // Goes over all received proofs and tries to update state
        // When  all requests were addressed and processed we reset the zkp_request variable

        while let Poll::Ready(Some((_peer_id, proof))) =
            this.zkp_requests.lock().poll_next_unpin(cx)
        {
            if let Err(e) = Self::do_push_proof(
                this.blockchain,
                this.zkp_state,
                proof,
                this.receiver,
                this.proof_generation_thread,
                this.proof_storage,
                true,
            ) {
                log::error!("Error pushing the new zk proof {}", e);
            }
        }

        // Check if new proof arrived, validate and store the new proofs.
        // Then, try to get as many blocks from the gossipsub stream as possible.
        // Stays with the first valid zproof it gets.
        loop {
            match this.zk_proofs_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(proof)) => {
                    log::debug!("Received zk proof via gossipsub {:?}", proof.0);
                    if let Err(e) = Self::do_push_proof(
                        this.blockchain,
                        this.zkp_state,
                        proof.0,
                        this.receiver,
                        this.proof_generation_thread,
                        this.proof_storage,
                        true,
                    ) {
                        log::error!("Error pushing the new zk proof {}", e);
                    }
                }
                Poll::Ready(None) => {}
                // If the zkp_stream is exhausted, we quit as well.
                _ => break,
            }
        }

        // If a new proof was generated it sets the state and broadcasts the new proof
        // This returns early to avoid multiple spawns of proof generation
        if let Some(ref mut receiver) = this.receiver {
            return match receiver.poll_unpin(cx) {
                Poll::Ready(Ok(Ok(zkp_state))) => {
                    *this.receiver = None;
                    let mut zkp_state_lock = this.zkp_state.write();
                    *zkp_state_lock = zkp_state;

                    let zkp_state_lock = RwLockWriteGuard::downgrade(zkp_state_lock);
                    assert!(
                        zkp_state_lock.latest_proof.is_some(),
                        "The generate new proof should never produces a empty proof"
                    );
                    let zkp_proof = ZKProof::new(
                        zkp_state_lock.latest_block_number,
                        zkp_state_lock.latest_proof.clone(),
                    );

                    Self::broadcast_zk_proof(this.network, zkp_proof);
                    Poll::Pending
                }
                Poll::Ready(Ok(Err(e))) => {
                    log::error!("Error generating ZK Proof for block {}", e);
                    *this.receiver = None;
                    Poll::Pending
                }
                _ => Poll::Pending,
            };
        }

        // Only active if the proof generation was activated on the constructor
        // Listens to block election events from blockchain and deploys the proof generation thread
        if let Some(ref mut blockchain_election_rx) = this.blockchain_election_rx {
            while let Poll::Ready(event) = blockchain_election_rx.poll_next_unpin(cx) {
                match event {
                    Some(BlockchainEvent::EpochFinalized(hash)) => {
                        let block = this.blockchain.read().get_block(&hash, true, None);
                        if let Some(Block::Macro(block)) = block {
                            let (sender, receiver) = oneshot::channel();
                            *this.receiver = Some(receiver);
                            let zkp_state = this.zkp_state.read();
                            *this.proof_generation_thread =
                                Some(tokio::spawn(ZKProofComponent::generate_new_proof(
                                    block,
                                    zkp_state.latest_pks.clone(),
                                    zkp_state.latest_header_hash.clone().into(),
                                    zkp_state.latest_proof.clone(),
                                    this.genesis_state.clone(),
                                    sender,
                                )));
                        }
                    }
                    Some(_) => (),
                    None => return Poll::Ready(()),
                }
            }
        }

        Poll::Pending
    }
}
