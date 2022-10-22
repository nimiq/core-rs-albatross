use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{FutureExt, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_genesis::NetworkInfo;
use nimiq_nano_primitives::state_commitment;
use nimiq_network_interface::network::Network;
use parking_lot::lock_api::RwLockUpgradableReadGuard;
use parking_lot::{RwLock, RwLockWriteGuard};

use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_utils::observer::NotifierStream;
use tokio::sync::oneshot::{self, Receiver};

use pin_project::pin_project;
use tokio::task::JoinHandle;

use crate::proof_component::*;
use crate::types::*;

/// Election blocks ZKP Component. Has:
///
/// - The previous election block public keys
/// - Previous election block header hash
/// - The pk tree root at genesis or none if genesis block
///
/// The proofs are returned by polling the components.
#[pin_project]
pub struct ZKProver<N: Network> {
    blockchain: Arc<RwLock<Blockchain>>,
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    genesis_state: Vec<u8>,
    receiver: Option<Receiver<Result<ZKPState, ZKPComponentError>>>,
    blockchain_election_rx: NotifierStream<BlockchainEvent>,
    proof_generation_thread: Option<JoinHandle<()>>,
}

impl<N: Network> ZKProver<N> {
    pub async fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<N>,
        zkp_state: Arc<RwLock<ZKPState>>,
    ) -> Self {
        let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
        let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();

        let public_keys = zkp_state.read().latest_pks.clone();
        let genesis_state = state_commitment(
            genesis_block.block_number(),
            genesis_block.hash().into(),
            public_keys,
        );

        let blockchain_election_rx = blockchain.write().notifier.as_stream();

        Self {
            blockchain,
            network,
            zkp_state,
            genesis_state,
            receiver: None,
            blockchain_election_rx,
            proof_generation_thread: None,
        }
    }

    pub fn is_generating_proof(&self) -> bool {
        self.proof_generation_thread.is_some()
    }

    fn broadcast_zk_proof(network: &Arc<N>, zk_proof: ZKProof) {
        let network = Arc::clone(network);
        tokio::spawn(async move {
            if let Err(e) = network.publish::<ZKProofTopic>(zk_proof).await {
                log::warn!(error = &e as &dyn Error, "Failed to publish the zk proof");
            }
        });
    }

    pub(crate) fn cancel_current_proof_production(&mut self) {
        self.receiver = None;
        if let Some(ref thread) = self.proof_generation_thread {
            thread.abort();
            self.proof_generation_thread = None;
        }
    }
}

impl<N: Network> Stream for ZKProver<N> {
    type Item = ZKProof;

    fn poll_next(mut self: Pin<&mut ZKProver<N>>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // If a new proof was generated it sets the state and broadcasts the new proof
        // This returns early to avoid multiple spawns of proof generation
        if let Some(ref mut receiver) = self.receiver {
            return match receiver.poll_unpin(cx) {
                Poll::Ready(Ok(Ok(new_zkp_state))) => {
                    assert!(
                        new_zkp_state.latest_proof.is_some(),
                        "The generate new proof should never produces a empty proof"
                    );
                    self.receiver = None;
                    let zkp_state_lock = self.zkp_state.upgradable_read();
                    if zkp_state_lock.latest_block_number < new_zkp_state.latest_block_number {
                        let mut zkp_state_lock = RwLockUpgradableReadGuard::upgrade(zkp_state_lock);
                        *zkp_state_lock = new_zkp_state;

                        let zkp_state_lock = RwLockWriteGuard::downgrade(zkp_state_lock);

                        let proof: ZKProof = zkp_state_lock.clone().into();
                        Self::broadcast_zk_proof(&self.network, proof.clone());
                        return Poll::Ready(Some(proof));
                    }
                    Poll::Pending //ITODO Doesn't need to return early but it's cleared to do so (?)
                }
                Poll::Ready(Ok(Err(e))) => {
                    log::error!("Error generating ZK Proof for block {}", e);
                    self.receiver = None;
                    Poll::Pending
                }
                _ => Poll::Pending,
            };
        }

        // Listens to block election events from blockchain and deploys the proof generation thread
        while let Poll::Ready(event) = self.blockchain_election_rx.poll_next_unpin(cx) {
            match event {
                Some(BlockchainEvent::EpochFinalized(hash)) => {
                    let block = self.blockchain.read().get_block(&hash, true, None);
                    if let Some(Block::Macro(block)) = block {
                        assert!(
                            self.proof_generation_thread.is_none() && self.receiver.is_none(),
                            "There should be no proof generation ongoing at this point."
                        );

                        let zkp_state = self.zkp_state.read().clone();
                        let (sender, receiver) = oneshot::channel();

                        self.receiver = Some(receiver);
                        self.proof_generation_thread = Some(tokio::spawn(generate_new_proof(
                            block,
                            zkp_state.latest_pks.clone(),
                            zkp_state.latest_header_hash.clone().into(),
                            zkp_state.latest_proof.clone(),
                            self.genesis_state.clone(),
                            sender,
                        )));
                    }
                }
                Some(_) => (),
                None => return Poll::Ready(None),
            }
        }

        Poll::Pending
    }
}
