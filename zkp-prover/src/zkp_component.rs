use std::path::PathBuf;
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

use crate::proof_utils::*;
use crate::types::*;
use crate::zkp_prover::ZKProver;
use crate::zkp_requests::ZKPRequests;
use futures::stream::BoxStream;

pub type ZKProofsStream<N> = BoxStream<'static, (ZKProof, <N as Network>::PubsubId)>;

pub(crate) const BROADCAST_MAX_CAPACITY: usize = 256;

pub struct ZKPComponentProxy<N: Network> {
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
    pub(crate) zkp_events_notifier: BroadcastSender<ZKProof>,
}

impl<N: Network> Clone for ZKPComponentProxy<N> {
    fn clone(&self) -> Self {
        Self {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            zkp_requests: Arc::clone(&self.zkp_requests),
            zkp_events_notifier: self.zkp_events_notifier.clone(),
        }
    }
}

impl<N: Network> ZKPComponentProxy<N> {
    /// Gets current zkp state.
    pub fn get_zkp_state(&self) -> ZKPState {
        self.zkp_state.read().clone()
    }

    /// Sends zkp request to all given peers. If no requests are ongoing, we request and return true,
    /// otherwise no requests will be sent.
    pub fn request_zkp_from_peers(&mut self, peers: Vec<N::PeerId>) -> bool {
        let mut zkp_requests_l = self.zkp_requests.lock();
        if zkp_requests_l.is_finished() {
            zkp_requests_l.request_zkps(peers, self.zkp_state.read().latest_block_number);
            return true;
        }
        false
    }

    pub fn subscribe_zkps(&self) -> BroadcastStream<ZKProof> {
        BroadcastStream::new(self.zkp_events_notifier.subscribe())
    }
}

/// ZKP Component aggregates the logic of request new proofs from peers, gossiping with peers on the most recent proofs,
/// pushing the received or generated zk proofs into state and storing them on the db.
///
/// The ZKP Compoenet has:
///
/// - The blockchain
/// - The network
/// - The current zkp state
/// - The proof generating component that can be activated by a client configuration
/// - The zkp gossip stream
/// - The db storage for the current proof
/// - The zkp requests component to fetch an up to date proof from our peers
/// - The zkp events notifies newly stored proofs.
///
/// Awaiting this future ensures that the zkp component works, this component should run forever.
#[pin_project]
pub struct ZKPComponent<N: Network> {
    blockchain: Arc<RwLock<Blockchain>>,
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    zk_prover: Option<ZKProver<N>>,
    zk_proofs_stream: ZKProofsStream<N>,
    proof_storage: ProofStore,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
    zkp_events_notifier: BroadcastSender<ZKProof>,
}

impl<N: Network> ZKPComponent<N> {
    pub async fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        is_prover_active: bool,
        prover_path: Option<PathBuf>,
        env: Environment,
    ) -> Self {
        // Defaults zkp state to genesis.
        let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
        let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();
        let zkp_state = Arc::new(RwLock::new(
            ZKPState::with_genesis(&genesis_block).expect("Invalid genesis block"),
        ));

        // Creates the zk proofs events notifier.
        let (zkp_events_notifier, _rx) = broadcast(BROADCAST_MAX_CAPACITY);

        // Loads the proof from the db if any.
        let proof_storage = ProofStore::new(env.clone());
        Self::load_proof_from_db(
            &blockchain,
            &zkp_state,
            &proof_storage,
            &zkp_events_notifier,
        );

        // Activates the prover based on the configuration provided.
        let zk_prover = if is_prover_active {
            Some(
                ZKProver::new(
                    Arc::clone(&blockchain),
                    Arc::clone(&network),
                    Arc::clone(&zkp_state),
                    prover_path,
                )
                .await,
            )
        } else {
            None
        };

        // Gets the stream zkps gossiped by peers.
        let zk_proofs_stream = network.subscribe::<ZKProofTopic>().await.unwrap().boxed();

        let zkp_component = Self {
            blockchain,
            network: Arc::clone(&network),
            zkp_state,
            zk_prover,
            zk_proofs_stream,
            proof_storage,
            zkp_requests: Arc::new(Mutex::new(ZKPRequests::new(network))),
            zkp_events_notifier,
        };

        // The handler for zkp request is launched.
        zkp_component.launch_request_handler();
        zkp_component
    }

    /// Launches thread that processes the zkp requests and replies to them.
    fn launch_request_handler(&self) {
        let stream = self.network.receive_requests::<RequestZKP>();
        tokio::spawn(request_handler(&self.network, stream, &self.zkp_state));
    }

    /// Gets a proxy for the current ZKP Component.
    pub fn proxy(&self) -> ZKPComponentProxy<N> {
        ZKPComponentProxy {
            network: Arc::clone(&self.network),
            zkp_state: Arc::clone(&self.zkp_state),
            zkp_requests: Arc::clone(&self.zkp_requests),
            zkp_events_notifier: self.zkp_events_notifier.clone(),
        }
    }

    /// Returns if the prover is activated.
    pub fn is_zkp_prover_activated(&self) -> bool {
        self.zk_prover.is_some()
    }

    /// Loads the proof from the database into the current state. It does all verification steps before loading it into
    /// our state. In case of failure, it replaces the db proof with the current state.
    fn load_proof_from_db(
        blockchain: &Arc<RwLock<Blockchain>>,
        zkp_state: &Arc<RwLock<ZKPState>>,
        proof_storage: &ProofStore,
        zkp_events_notifier: &BroadcastSender<ZKProof>,
    ) {
        if let Some(loaded_proof) = proof_storage.get_zkp() {
            if let Err(e) = Self::push_proof_from_peers(
                blockchain,
                zkp_state,
                loaded_proof,
                &mut None,
                proof_storage,
                zkp_events_notifier,
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

    /// Pushes the proof sent from an peer into our own state. If the proof is invalid or it's older than the
    /// current state it fails.
    fn push_proof_from_peers(
        blockchain: &Arc<RwLock<Blockchain>>,
        zkp_state: &Arc<RwLock<ZKPState>>,
        zk_proof: ZKProof,
        zk_prover: &mut Option<ZKProver<N>>,
        proof_storage: &ProofStore,
        zkp_events_notifier: &BroadcastSender<ZKProof>,
        add_to_storage: bool,
    ) -> Result<(), ZKPComponentError> {
        // Gets the relevant election blocks.
        let (new_block, genesis_block, proof) = get_proof_macro_blocks(blockchain, &zk_proof)?;
        let zkp_state_lock = zkp_state.upgradable_read();

        // Ensures that the proof is more recent than our current state and validates the proof.
        if new_block.block_number() <= zkp_state_lock.latest_block_number {
            return Err(ZKPComponentError::OutdatedProof);
        }
        let new_zkp_state = validate_proof_get_new_state(proof, new_block, genesis_block)?;

        let mut zkp_state_lock = RwLockUpgradableReadGuard::upgrade(zkp_state_lock);
        *zkp_state_lock = new_zkp_state;

        // Since someone else generate a valid proof faster, we will terminate our own proof generation process.
        if let Some(zk_prover) = zk_prover {
            zk_prover.cancel_current_proof_production();
        }

        // Adds the new proof to storage.
        if add_to_storage {
            proof_storage.set_zkp(&zkp_state_lock.clone().into())
        }

        // Sends the new event to the notifier stream.
        if let Err(e) = zkp_events_notifier.send(zk_proof) {
            log::error!("Error sending the proof to stream {}", e);
        }

        Ok(())
    }
}

impl<N: Network> Future for ZKPComponent<N> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();

        // Check if we have requested zkps rom peers. Goes over all received proofs and tries to update state.
        // Stays with the first most recent valid zproof it gets.
        while let Poll::Ready(Some((_peer_id, proof))) =
            this.zkp_requests.lock().poll_next_unpin(cx)
        {
            if let Err(e) = Self::push_proof_from_peers(
                this.blockchain,
                this.zkp_state,
                proof,
                this.zk_prover,
                this.proof_storage,
                this.zkp_events_notifier,
                true,
            ) {
                log::error!("Error pushing the new zk proof {}", e);
            }
        }

        // Exhaustes all peer gossiped proofs and tries to push them.
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
                        this.zkp_events_notifier,
                        true,
                    ) {
                        log::error!("Error pushing the zk proof - {} ", e);
                    }
                }
                Poll::Ready(None) => {
                    // The stream was closed so we quit as well.
                    log::error!("ZK gossip stream aborted.");
                    return Poll::Ready(());
                }
                _ => {
                    // If the zkp_stream is exhausted, we stop polling.
                    break;
                }
            }
        }

        // Polls prover for new proofs and notifies the new event to the zkp events notifier.
        if let Some(ref mut zk_prover) = this.zk_prover {
            match zk_prover.poll_next_unpin(cx) {
                Poll::Ready(Some(zk_proof)) => {
                    log::info!("New ZK Proof generated by us");
                    if let Err(e) = this.zkp_events_notifier.send(zk_proof) {
                        log::error!("Error sending the proof to stream {}", e);
                    }
                }
                Poll::Ready(None) => {
                    // The stream was closed so we quit as well.
                    log::error!("ZK prover aborted. We are no longer generating new proofs");
                    return Poll::Ready(());
                }
                _ => {}
            };
        }

        Poll::Pending
    }
}
