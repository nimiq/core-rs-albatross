use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Future, StreamExt};
use nimiq_block::Block;
use nimiq_database::Environment;
use nimiq_genesis::NetworkInfo;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_network_interface::{network::Network, request::request_handler};

use pin_project::pin_project;
use tokio::sync::broadcast::{channel as broadcast, Sender as BroadcastSender};
use tokio_stream::wrappers::BroadcastStream;

use crate::proof_component::*;
use crate::types::*;
use crate::zkp_prover::ZKProver;
use crate::zkp_requests::ZKPRequests;
use futures::stream::BoxStream;

pub type ZKProofsStream<N> = BoxStream<'static, (ZKProof, <N as Network>::PubsubId)>;

const BROADCAST_MAX_CAPACITY: usize = 256;

pub struct ZKPComponentProxy<N: Network> {
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
    pub(crate) zkp_events: BroadcastSender<ZKProof>,
}

impl<N: Network> Clone for ZKPComponentProxy<N> {
    fn clone(&self) -> Self {
        Self {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            zkp_requests: Arc::clone(&self.zkp_requests),
            zkp_events: self.zkp_events.clone(),
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

    pub fn subscribe_zkps(&self) -> BroadcastStream<ZKProof> {
        BroadcastStream::new(self.zkp_events.subscribe())
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
    zk_prover: Option<ZKProver<N>>,
    zk_proofs_stream: ZKProofsStream<N>,
    proof_storage: ProofStore,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
    zkp_events: BroadcastSender<ZKProof>,
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

        let zkp_state = Arc::new(RwLock::new(
            ZKPState::with_genesis(&genesis_block).expect("Invalid genesis block"),
        ));

        let (zkp_events, _rx) = broadcast(BROADCAST_MAX_CAPACITY);

        // Load from db the preexisting proof if any
        let proof_storage = ProofStore::new(env.clone());
        Self::load_proof_from_db(&blockchain, &zkp_state, &proof_storage, &zkp_events);

        let zk_prover = if is_prover_active {
            Some(
                ZKProver::new(
                    Arc::clone(&blockchain),
                    Arc::clone(&network),
                    Arc::clone(&zkp_state),
                )
                .await,
            )
        } else {
            None
        };

        // Sets the stream of the broadcasted
        let zk_proofs_stream = network.subscribe::<ZKProofTopic>().await.unwrap().boxed();

        let zkp_component = Self {
            blockchain,
            network: Arc::clone(&network),
            zkp_state,
            zk_prover,
            zk_proofs_stream,
            proof_storage,
            zkp_requests: Arc::new(Mutex::new(ZKPRequests::new(network))),
            zkp_events,
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
            zkp_requests: Arc::clone(&self.zkp_requests),
            zkp_events: self.zkp_events.clone(),
        }
    }

    pub fn is_zkp_prover_activated(&self) -> bool {
        self.zk_prover.is_some()
    }

    fn load_proof_from_db(
        blockchain: &Arc<RwLock<Blockchain>>,
        zkp_state: &Arc<RwLock<ZKPState>>,
        proof_storage: &ProofStore,
        zkp_events: &BroadcastSender<ZKProof>,
    ) {
        // Load from db the preexisting proof if any.
        if let Some(loaded_proof) = proof_storage.get_zkp() {
            if let Err(e) = Self::push_proof_from_peers(
                blockchain,
                zkp_state,
                loaded_proof,
                &mut None,
                proof_storage,
                zkp_events,
                false,
            ) {
                log::error!("Error pushing the zk proof load from disk {}", e);
                proof_storage.set_zkp(&zkp_state.read().clone().into());
            } else {
                log::info!("The zk proof was successfully load from disk");
            }
        } else {
            log::trace!("No zk proof found on the db");
        }
    }

    fn push_proof_from_peers(
        blockchain: &Arc<RwLock<Blockchain>>,
        zkp_state: &Arc<RwLock<ZKPState>>,
        zk_proof: ZKProof,
        zk_prover: &mut Option<ZKProver<N>>,
        proof_storage: &ProofStore,
        zkp_events: &BroadcastSender<ZKProof>,
        add_to_storage: bool,
    ) -> Result<(), ZKPComponentError> {
        let (new_block, genesis_block, proof) = pre_proof_validity_checks(blockchain, &zk_proof)?;
        let zkp_state_lock = zkp_state.upgradable_read();

        if new_block.block_number() <= zkp_state_lock.latest_block_number {
            return Err(ZKPComponentError::OutdatedProof);
        }
        let new_zkp_state = validate_proof_get_new_state(proof, new_block, genesis_block)?;

        let mut zkp_state_lock = RwLockUpgradableReadGuard::upgrade(zkp_state_lock);
        *zkp_state_lock = new_zkp_state;

        // Since someone else generate a valide proof faster, we will terminate our own proof generation.
        if let Some(zk_prover) = zk_prover {
            zk_prover.cancel_current_proof_production();
        }

        if add_to_storage {
            proof_storage.set_zkp(&zkp_state_lock.clone().into())
        }

        if let Err(e) = zkp_events.send(zk_proof) {
            log::error!("Error sending the proof to stream {}", e);
        }

        Ok(())
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
            if let Err(e) = Self::push_proof_from_peers(
                this.blockchain,
                this.zkp_state,
                proof,
                this.zk_prover,
                this.proof_storage,
                this.zkp_events,
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
                    if let Err(e) = Self::push_proof_from_peers(
                        this.blockchain,
                        this.zkp_state,
                        proof.0,
                        this.zk_prover,
                        this.proof_storage,
                        this.zkp_events,
                        true,
                    ) {
                        log::error!("Error pushing the zk proof - {} ", e);
                    }
                }
                Poll::Ready(None) => {}
                // If the zkp_stream is exhausted, we quit as well.
                _ => break,
            }
        }

        // Polls prover for new proofs
        if let Some(ref mut zk_prover) = this.zk_prover {
            match zk_prover.poll_next_unpin(cx) {
                Poll::Ready(Some(_zkp_state)) => {
                    //ITODO info message
                }
                Poll::Ready(None) => {
                    log::error!("ZK prover aborted. We are no longer generating new proofs");
                    return Poll::Ready(());
                }
                _ => {}
            };
        }

        Poll::Pending
    }
}
