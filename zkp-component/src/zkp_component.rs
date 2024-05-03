#[cfg(feature = "zkp-prover")]
use std::path::PathBuf;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{stream::BoxStream, Future, StreamExt};
use nimiq_block::MacroBlock;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_genesis::NetworkInfo;
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, PubsubId},
    request::request_handler,
};
use nimiq_primitives::task_executor::TaskExecutor;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use tokio::sync::{
    broadcast::{channel as broadcast, Sender as BroadcastSender},
    oneshot::error::RecvError,
};
use tokio_stream::wrappers::BroadcastStream;

#[cfg(feature = "zkp-prover")]
use crate::zkp_prover::ZKProver;
use crate::{proof_store::ProofStore, proof_utils::*, types::*, zkp_requests::ZKPRequests};

pub type ZKProofsStream<N> = BoxStream<'static, (ZKProof, <N as Network>::PubsubId)>;

pub(crate) const BROADCAST_MAX_CAPACITY: usize = 256;

pub struct ZKPComponentProxy<N: Network> {
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
    pub(crate) zkp_events_notifier: BroadcastSender<ZKPEvent<N>>,
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
    pub fn request_zkp_from_peers(
        &self,
        peers: Vec<N::PeerId>,
        request_election_block: bool,
    ) -> bool {
        let mut zkp_requests_l = self.zkp_requests.lock();
        if zkp_requests_l.is_finished() {
            zkp_requests_l.request_zkps(
                peers,
                self.zkp_state.read().latest_block.block_number(),
                request_election_block,
            );
            return true;
        }
        false
    }

    pub async fn request_zkp_from_peer(
        &self,
        peer_id: N::PeerId,
        request_election_block: bool,
    ) -> (Result<Result<ZKPRequestEvent, Error>, RecvError>, N::PeerId) {
        let block_number = self.zkp_state.read().latest_block.block_number();
        let request =
            self.zkp_requests
                .lock()
                .request_zkp(peer_id, block_number, request_election_block);
        (request.await, peer_id)
    }

    pub fn subscribe_zkps(&self) -> BroadcastStream<ZKPEvent<N>> {
        BroadcastStream::new(self.zkp_events_notifier.subscribe())
    }
}

/// ZKP Component aggregates the logic of request new proofs from peers, gossiping with peers on the most recent proofs,
/// pushing the received or generated zk proofs into state and storing them on the db.
///
/// The ZKP Component has:
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
pub struct ZKPComponent<N: Network> {
    pub(crate) blockchain: BlockchainProxy,
    network: Arc<N>,
    pub(crate) zkp_state: Arc<RwLock<ZKPState>>,
    #[cfg(feature = "zkp-prover")]
    zk_prover: Option<ZKProver<N>>,
    zk_proofs_stream: ZKProofsStream<N>,
    proof_storage: Option<Box<dyn ProofStore>>,
    zkp_requests: Arc<Mutex<ZKPRequests<N>>>,
    zkp_events_notifier: BroadcastSender<ZKPEvent<N>>,
}

impl<N: Network> ZKPComponent<N> {
    pub async fn new(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        executor: impl TaskExecutor + Send + 'static + std::marker::Sync,
        proof_storage: Option<Box<dyn ProofStore>>,
    ) -> Self {
        // Defaults zkp state to genesis.
        let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
        let genesis_block = network_info.genesis_block().unwrap_macro();
        let zkp_state = Arc::new(RwLock::new(
            ZKPState::with_genesis(&genesis_block).expect("Invalid genesis block"),
        ));

        // Creates the zk proofs events notifier.
        let (zkp_events_notifier, _rx) = broadcast(BROADCAST_MAX_CAPACITY);

        // Gets the stream zkps gossiped by peers.
        let zk_proofs_stream = network.subscribe::<ZKProofTopic>().await.unwrap().boxed();

        let mut zkp_component = Self {
            blockchain,
            network: Arc::clone(&network),
            zkp_state,
            #[cfg(feature = "zkp-prover")]
            zk_prover: None,
            zk_proofs_stream,
            proof_storage,
            zkp_requests: Arc::new(Mutex::new(ZKPRequests::new(network))),
            zkp_events_notifier,
        };

        // Loads the proof from the db if any.
        zkp_component.load_proof_from_db();

        // The handler for zkp request is launched.
        zkp_component.launch_request_handler(executor);
        zkp_component
    }

    #[cfg(feature = "zkp-prover")]
    pub async fn with_prover(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        executor: impl TaskExecutor + Send + 'static + std::marker::Sync,
        is_prover_active: bool,
        prover_path: Option<PathBuf>,
        prover_keys_path: PathBuf,
        proof_storage: Option<Box<dyn ProofStore>>,
    ) -> Self {
        let mut zkp_component = Self::new(blockchain, network, executor, proof_storage).await;

        // Activates the prover based on the configuration provided.
        zkp_component.zk_prover = match (is_prover_active, &zkp_component.blockchain) {
            (true, BlockchainProxy::Full(ref blockchain)) => Some(
                ZKProver::new(
                    Arc::clone(blockchain),
                    Arc::clone(&zkp_component.network),
                    Arc::clone(&zkp_component.zkp_state),
                    prover_path,
                    prover_keys_path,
                )
                .await,
            ),
            (true, _) => {
                log::error!("ZKP Prover cannot be activated for a light node.");
                None
            }
            _ => None,
        };

        zkp_component
    }

    /// Launches thread that processes the zkp requests and replies to them.
    fn launch_request_handler(
        &self,
        executor: impl TaskExecutor + Send + 'static + std::marker::Sync,
    ) {
        let stream = self.network.receive_requests::<RequestZKP>();
        let env = Arc::new(ZKPStateEnvironment::from(self));
        executor.exec(Box::pin(request_handler(
            &self.network,
            executor.clone(),
            stream,
            &env,
        )));
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
        #[cfg(feature = "zkp-prover")]
        return self.zk_prover.is_some();
        #[cfg(not(feature = "zkp-prover"))]
        false
    }

    /// Loads the proof from the database into the current state. It does all verification steps before loading it into
    /// our state. In case of failure, it replaces the db proof with the current state.
    fn load_proof_from_db(&mut self) {
        if let Some(proof_storage) = &self.proof_storage {
            if let Some(loaded_proof) = proof_storage.get_zkp() {
                let mut this = Pin::new(self);

                if let Err(e) = this.as_mut().push_proof_from_peers(
                    loaded_proof,
                    None,
                    false,
                    ProofSource::SelfGenerated,
                ) {
                    log::error!("Error pushing the zk proof load from disk {}", e);
                    this.proof_storage
                        .as_ref()
                        .unwrap()
                        .set_zkp(&this.zkp_state.read().clone().into());
                } else {
                    log::info!("The zk proof was successfully load from disk");
                }
            } else {
                log::info!("No zk proof found on the db");
            }
        }
    }

    /// Pushes the proof sent from an peer into our own state. If the proof is invalid or it's older than the
    /// current state it fails.
    fn push_proof_from_peers(
        mut self: Pin<&mut Self>,
        zk_proof: ZKProof,
        election_block: Option<MacroBlock>,
        add_to_storage: bool,
        proof_source: ProofSource<N>,
    ) -> Result<ZKPEvent<N>, Error> {
        // Gets the relevant election blocks.
        let (new_block, genesis_block, proof) =
            get_proof_macro_blocks(&self.blockchain, &zk_proof, election_block)?;
        let zkp_state_lock = self.zkp_state.upgradable_read();

        // Ensures that the proof is more recent than our current state and validates the proof.
        if new_block.block_number() <= zkp_state_lock.latest_block.block_number() {
            return Err(Error::OutdatedProof);
        }
        let new_zkp_state = validate_proof_get_new_state(proof, new_block.clone(), genesis_block)?;

        let mut zkp_state_lock = RwLockUpgradableReadGuard::upgrade(zkp_state_lock);
        *zkp_state_lock = new_zkp_state;

        // Adds the new proof to storage.
        if let Some(proof_storage) = &self.proof_storage {
            if add_to_storage {
                proof_storage.set_zkp(&zkp_state_lock.clone().into())
            }
        }
        drop(zkp_state_lock);

        // Since someone else generate a valid proof faster, we will terminate our own proof generation process.
        #[cfg(feature = "zkp-prover")]
        if let Some(ref mut zk_prover) = self.zk_prover {
            zk_prover.cancel_current_proof_production();
        }

        // Sends the new event to the notifier stream.
        let event = ZKPEvent::new(proof_source, zk_proof, new_block);
        _ = self.zkp_events_notifier.send(event.clone());

        Ok(event)
    }
}

impl<N: Network> Future for ZKPComponent<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        // Check if we have requested zkps rom peers. Goes over all received proofs and tries to update state.
        // Stays with the first most recent valid zk proof it gets.
        loop {
            let zkp_result = self.zkp_requests.lock().poll_next_unpin(cx);
            match zkp_result {
                Poll::Ready(Some(requests_item)) => {
                    let result = self.as_mut().push_proof_from_peers(
                        requests_item.proof,
                        requests_item.election_block,
                        true,
                        ProofSource::PeerGenerated(requests_item.peer_id),
                    );

                    // Log errors.
                    match result {
                        Err(Error::OutdatedProof) => log::trace!("ZK Proof was outdated"),
                        Err(ref e) => log::error!("Error pushing the new zk proof {}", e),
                        _ => {}
                    }

                    // Return verification result if channel exists.
                    if let Some(tx) = requests_item.response_channel {
                        let _ = tx.send(result.map(|event| ZKPRequestEvent::Proof {
                            proof: event.proof,
                            block: event.block,
                        }));
                    }
                }
                _ => {
                    break;
                }
            }
        }

        // Exhausts all peer gossiped proofs and tries to push them.
        loop {
            match self.zk_proofs_stream.as_mut().poll_next_unpin(cx) {
                Poll::Ready(Some((proof, pub_id))) => {
                    log::debug!("Received zk proof via gossipsub {:?}", proof);
                    let acceptance = match self.as_mut().push_proof_from_peers(
                        proof,
                        None,
                        true,
                        ProofSource::PeerGenerated(pub_id.propagation_source()),
                    ) {
                        Err(Error::OutdatedProof) => {
                            log::trace!("ZK Proof was outdated");
                            MsgAcceptance::Ignore
                        }
                        Err(e) => {
                            log::error!("Error pushing the zk proof - {} ", e);
                            MsgAcceptance::Reject
                        }
                        Ok(_) => MsgAcceptance::Accept,
                    };
                    self.network
                        .validate_message::<ZKProofTopic>(pub_id, acceptance);
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
        #[cfg(feature = "zkp-prover")]
        while let Some(ref mut zk_prover) = self.zk_prover {
            match zk_prover.poll_next_unpin(cx) {
                Poll::Ready(Some((zk_proof, block))) => {
                    log::info!("New ZK Proof generated by us");
                    if let Some(proof_storage) = &self.proof_storage {
                        proof_storage.set_zkp(&self.zkp_state.read().clone().into());
                    }

                    _ = self.zkp_events_notifier.send(ZKPEvent::new(
                        ProofSource::SelfGenerated,
                        zk_proof,
                        block,
                    ));
                }
                Poll::Ready(None) => {
                    // The stream was closed so we quit as well.
                    log::error!("ZK prover aborted. We are no longer generating new proofs");
                    return Poll::Ready(());
                }
                Poll::Pending => break,
            };
        }

        Poll::Pending
    }
}
