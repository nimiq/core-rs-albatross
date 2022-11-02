use std::error::Error;
use std::future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use nimiq_block::Block;
use nimiq_genesis::NetworkInfo;
use nimiq_nano_primitives::state_commitment;
use nimiq_network_interface::network::Network;
use parking_lot::lock_api::RwLockUpgradableReadGuard;
use parking_lot::{RwLock, RwLockWriteGuard};

use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};

use tokio::sync::broadcast::{channel as broadcast, Sender as BroadcastSender};

use crate::proof_utils::*;
use crate::types::*;
use crate::zkp_component::BROADCAST_MAX_CAPACITY;

/// ZK Prover generates the zk proof for an election block. It has:
///
/// - The network
/// - The current zkp state
/// - The channel to kill the current process generating the proof
/// - The stream with the generated zero knowledge proofs
///
/// The proofs are returned by polling the components.
pub struct ZKProver<N: Network> {
    network: Arc<N>,
    zkp_state: Arc<RwLock<ZKPState>>,
    sender: BroadcastSender<()>,
    proof_stream: Pin<Box<dyn Stream<Item = Result<ZKPState, ZKProofGenerationError>> + Send>>,
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
        let (sender, recv) = broadcast(BROADCAST_MAX_CAPACITY);

        let blockchain2 = Arc::clone(&blockchain);
        let blockchain_election_rx = blockchain_election_rx.filter_map(move |event| {
            let result = match event {
                BlockchainEvent::EpochFinalized(hash) => {
                    let block = blockchain2.read().get_block(&hash, true, None);
                    if let Some(Block::Macro(block)) = block {
                        Some(block)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            future::ready(result)
        });

        let zkp_state2 = Arc::clone(&zkp_state);
        let proof_stream = blockchain_election_rx
            .then(move |block| {
                let genesis_state3 = genesis_state.clone();
                let zkp_state3 = Arc::clone(&zkp_state2);
                let recv = recv.resubscribe();
                let zkp_state = zkp_state3.read();
                assert_eq!(zkp_state.latest_header_hash, block.header.parent_election_hash, "The new block should be the immediatly after the one we currently have the proof for");
                launch_generate_new_proof(
                    recv,
                    ProofInput {
                        block,
                        latest_pks: zkp_state.latest_pks.clone(),
                        latest_header_hash: zkp_state.latest_header_hash.clone(),
                        previous_proof: zkp_state.latest_proof.clone(),
                        genesis_state: genesis_state3,
                    },
                )
            })
            .boxed();

        Self {
            network,
            zkp_state,
            sender,
            proof_stream,
        }
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
        self.sender.send(()).unwrap();
    }
}

impl<N: Network> Stream for ZKProver<N> {
    type Item = ZKProof;

    fn poll_next(mut self: Pin<&mut ZKProver<N>>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // If a new proof was generated it sets the state and broadcasts the new proof.
        while let Poll::Ready(Some(proof)) = self.proof_stream.poll_next_unpin(cx) {
            match proof {
                Ok(new_zkp_state) => {
                    assert!(
                        new_zkp_state.latest_proof.is_some(),
                        "The generate new proof should never produces a empty proof"
                    );
                    let zkp_state_lock = self.zkp_state.upgradable_read();
                    if zkp_state_lock.latest_block_number < new_zkp_state.latest_block_number {
                        let mut zkp_state_lock = RwLockUpgradableReadGuard::upgrade(zkp_state_lock);
                        *zkp_state_lock = new_zkp_state;

                        let zkp_state_lock = RwLockWriteGuard::downgrade(zkp_state_lock);

                        let proof: ZKProof = zkp_state_lock.clone().into();
                        Self::broadcast_zk_proof(&self.network, proof.clone());
                        return Poll::Ready(Some(proof));
                    }
                }
                Err(e) => {
                    log::error!("Error generating ZK Proof for block {}", e);
                }
            };
        }

        Poll::Pending
    }
}
