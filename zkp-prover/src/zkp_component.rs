use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{FutureExt, Stream, StreamExt};

use nimiq_block::{Block, MacroBlock};
use nimiq_database::Environment;
use nimiq_hash::Blake2bHash;
use nimiq_nano_primitives::{state_commitment, MacroBlock as ZKPMacroBlock};
use nimiq_network_interface::request::RequestError;
use parking_lot::{RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_network_interface::network::Network;
use nimiq_utils::observer::NotifierStream;
use tokio::sync::oneshot::{self, Receiver};

use pin_project::pin_project;
use tokio::task::JoinHandle;

use nimiq_consensus::consensus::Consensus;

use crate::proof_component::{self as ZKProofComponent, ProofStore};
use crate::types::*;
use futures::stream::BoxStream;

pub type ZKProofsStream<N> = BoxStream<'static, (ZKProof, <N as Network>::PubsubId)>;

pub struct ZKPComponentProxy<N: Network> {
    pub network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    genesis_state: Vec<u8>,
}

impl<N: Network> Clone for ZKPComponentProxy<N> {
    fn clone(&self) -> Self {
        Self {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            genesis_state: self.genesis_state.clone(),
        }
    }
}

impl<N: Network> ZKPComponentProxy<N> {
    pub async fn request_zkp(&self, peer_id: N::PeerId) -> Result<ZKProof, RequestError> {
        self.network
            .request::<RequestZKP>(RequestZKP {}, peer_id)
            .await
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
    pub(crate) zkp_state: Arc<RwLock<ZKPState>>,
    genesis_state: Vec<u8>,
    receiver: Option<Receiver<(Blake2bHash, Result<ZKPState, ZKPComponentError>)>>,
    blockchain_election_rx: Option<NotifierStream<BlockchainEvent>>,
    zk_proofs_stream: ZKProofsStream<N>,
    proof_generation_thread: Option<JoinHandle<()>>,
    proof_storage: ProofStore,
    //zkp_request_result: Option<Pin<Box<dyn Future<Output = Result<ZKProof, RequestError>>>>>,
}

impl<N: Network> ZKPComponent<N> {
    pub async fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        genesis_block: MacroBlock,
        is_prover_active: bool,
        env: Environment,
    ) -> Self {
        let latest_pks: Vec<_> = genesis_block //ITODO maybe call push proof instead
            .get_validators()
            .unwrap()
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect(); //itodo

        let genesis_hash = genesis_block.hash();

        let genesis_block = ZKPMacroBlock::try_from(&genesis_block).unwrap();
        let genesis_state = state_commitment(
            genesis_block.block_number,
            genesis_block.header_hash,
            latest_pks.clone(),
        );

        let mut blockchain_election_rx = None;
        if is_prover_active {
            blockchain_election_rx = Some(blockchain.write().notifier.as_stream());
        }
        let zk_proofs_stream = network.subscribe::<ZKProofTopic>().await.unwrap().boxed();

        // Sets to zkp_component state to genesis
        let mut zkp_state = Arc::new(RwLock::new(ZKPState {
            latest_pks,
            latest_header_hash: genesis_hash,
            latest_block_number: genesis_block.block_number,
            latest_proof: None,
        }));

        // Load from db the preexisting proof if any
        let proof_storage = ProofStore::new(env);
        if let Some(loaded_zkp_state) = proof_storage.get_zkp() {
            let loaded_zkp_state = Arc::new(RwLock::new(loaded_zkp_state));
            if let Err(e) = Self::do_push_proof(
                &blockchain,
                &loaded_zkp_state,
                loaded_zkp_state.read().into(),
                &mut None,
                &mut None,
                &proof_storage,
                false,
            ) {
                log::error!("Error pushing the zk proof load from disk {}", e);
                // ITODO Delete proof/push genesis proof (?) ask pascal
                proof_storage.set_zkp(&zkp_state.read());
            } else {
                zkp_state = loaded_zkp_state;
            }
        }

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
        };

        zkp_component.launch_request_handler();
        zkp_component
    }

    fn launch_request_handler(&self) {
        let stream = self.network.receive_requests::<RequestZKP>();
        tokio::spawn(Consensus::request_handler(
            &self.network,
            stream,
            &self.zkp_state,
        ));
    }

    pub fn proxy(&self) -> ZKPComponentProxy<N> {
        ZKPComponentProxy {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            genesis_state: self.genesis_state.clone(),
        }
    }

    pub async fn request_zkp(&self, peer_id: N::PeerId) -> Result<ZKProof, RequestError> {
        self.network
            .request::<RequestZKP>(RequestZKP {}, peer_id)
            .await
    }

    pub fn make_request_zkp(&mut self, _peer_id: N::PeerId) {
        //let network = Arc::clone(&self.network);
        //let stuff = N::request::<RequestZKP>(network, RequestZKP {}, peer_id);
        //self.zkp_request_result = Some(stuff);
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
        receiver: &mut Option<Receiver<(Blake2bHash, Result<ZKPState, ZKPComponentError>)>>,
        proof_generation_thread: &mut Option<JoinHandle<()>>,
        proof_storage: &ProofStore,
        add_to_storage: bool,
    ) -> Result<(), ZKPComponentError> {
        let (new_block, old_block, proof) =
            ZKProofComponent::pre_proof_validity_checks(blockchain, proof)?;

        let zkp_state_lock = zkp_state.upgradable_read();
        if new_block.block_number() <= zkp_state_lock.latest_block_number {
            return Err(ZKPComponentError::OutdatedProof);
        }

        if let Ok(Some(new_zkp_state)) =
            ZKProofComponent::validate_proof_get_new_state(proof, new_block, old_block)
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

impl<N: Network> Stream for ZKPComponent<N> {
    type Item = ZKProof;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.project();

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
                Poll::Ready(None) => {} //Error proof (?)
                // If the zkp_stream is exhausted, we quit as well.
                _ => break,
            }
        }

        // This returns early to avoid multiple spawns of proof generation.
        if let Some(ref mut receiver) = this.receiver {
            return match receiver.poll_unpin(cx) {
                Poll::Ready(Ok((block_hash, Ok(zkp_state)))) => {
                    *this.receiver = None;
                    let mut zkp_state_lock = this.zkp_state.write();
                    *zkp_state_lock = zkp_state;

                    let zkp_state_lock = RwLockWriteGuard::downgrade(zkp_state_lock);
                    let zkp_proof = ZKProof::new(
                        block_hash,
                        zkp_state_lock.latest_proof.clone(), //.ok_or(NanoZKPError::EmptyProof),
                    );

                    Self::broadcast_zk_proof(this.network, zkp_proof.clone());
                    Poll::Ready(Some(zkp_proof))
                }
                Poll::Ready(Ok((block_hash, Err(e)))) => {
                    *this.receiver = None;
                    log::error!("Error generating ZK Proof {}", e);
                    Poll::Ready(Some(ZKProof::new(block_hash, None)))
                }
                _ => Poll::Pending,
            };
        }

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
                    None => return Poll::Ready(None),
                }
            }
        }

        Poll::Pending
    }
}
