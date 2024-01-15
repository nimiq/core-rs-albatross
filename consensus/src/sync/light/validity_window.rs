use std::{
    cmp,
    sync::Arc,
    task::{Context, Poll},
};

use futures::StreamExt;
#[cfg(feature = "full")]
use nimiq_blockchain::{Blockchain, CHUNK_SIZE};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_mmr::mmr::position::leaf_number_to_index;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    request::RequestError,
};
use nimiq_primitives::policy::Policy;

use super::LightMacroSync;
#[cfg(feature = "full")]
use crate::messages::{HistoryChunk, HistoryChunkError, RequestHistoryChunk};
use crate::sync::{light::sync::ValidityChunkRequest, syncer::MacroSyncReturn};

/// The validity start is defined as the current head - validity_window_blocks
/// However, if the validity start is located in the current epoch
/// we will move it to the beggining of the current epoch,
/// in order to sync the full epoch and being able to verify the history root.
pub fn compute_validity_start(macro_head: u32) -> u32 {
    // First we determine which is the validity window start block number
    let mut validity_window_bn = cmp::max(
        Policy::genesis_block_number(),
        macro_head.saturating_sub(Policy::transaction_validity_window_blocks()),
    );

    // Now we need to check which is the election block after the validity start to determine
    // if the full validity window is located in the current epoch.
    let next_election = Policy::election_block_after(validity_window_bn);

    // If this condition is true it means that the validity window is located within the current epoch
    if next_election > macro_head && !Policy::is_election_block_at(validity_window_bn) {
        // So we move the validity start to the beggining of the epoch, in order to be able to verify the history root
        validity_window_bn = Policy::election_block_before(validity_window_bn)
    };

    validity_window_bn
}

#[cfg(feature = "full")]
impl<TNetwork: Network> LightMacroSync<TNetwork> {
    pub async fn request_validity_window_chunk(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        epoch_number: u32,
        block_number: u32,
        chunk_index: u64,
    ) -> Result<Result<HistoryChunk, HistoryChunkError>, RequestError> {
        // A validity window chunk is simply a history chunk
        network
            .request::<RequestHistoryChunk>(
                RequestHistoryChunk {
                    epoch_number,
                    block_number,
                    chunk_index,
                },
                peer_id,
            )
            .await
    }

    fn start_validity_chunk_request(
        &mut self,
        peer_id: TNetwork::PeerId,
        verifier_block_number: u32,
        expected_root: Blake2bHash,
        validity_window_start: u32,
        election_in_window: bool,
    ) {
        if self.validity_requests.is_none() {
            // In this case we are going to start a new request
            let epoch_number = Policy::epoch_at(verifier_block_number);
            let chunk_index = 0;

            self.validity_requests = Some(ValidityChunkRequest {
                verifier_block_number,
                root_hash: expected_root,
                chunk_index,
                initial_offset: chunk_index * CHUNK_SIZE as u32,
                validity_start: validity_window_start,
                election_in_window,
            });

            // Add the peer
            self.validity_queue.add_peer(peer_id);
            self.syncing_peers.insert(peer_id);

            let request = RequestHistoryChunk {
                epoch_number,
                block_number: verifier_block_number,
                chunk_index: chunk_index as u64,
            };

            log::debug!(
                verifier_bn = verifier_block_number,
                chunk_index = chunk_index,
                epoch = epoch_number,
                validity_start = validity_window_start,
                election_in_between = election_in_window,
                "Starting validity window synchronization process"
            );

            // If we already have some history, we need to do a cleanup before starting the sync process
            match &self.blockchain {
                BlockchainProxy::Full(blockchain) => {
                    let mut blockchain_wr = blockchain.write();
                    if blockchain_wr
                        .history_store
                        .length_at(verifier_block_number, None)
                        > 0
                    {
                        blockchain_wr.remove_epoch_history(epoch_number);

                        log::debug!(epoch = epoch_number, "Removing existing history from epoch");
                    }
                }
                BlockchainProxy::Light(_) => {
                    unreachable!()
                }
            };

            // Request the chunk
            self.validity_queue.add_ids(vec![(request, None)]);
        } else {
            // If we are already requesting chunks, then we only add the peer
            self.validity_queue.add_peer(peer_id);
            self.syncing_peers.insert(peer_id);
        }
    }

    pub fn start_validity_synchronization(&mut self, peer_id: TNetwork::PeerId) {
        let macro_head = self.blockchain.read().macro_head();
        let validity_window_bn = compute_validity_start(macro_head.block_number());

        // This must correspond to a macro block.
        assert!(Policy::is_macro_block_at(validity_window_bn));

        log::trace!(
            macro_head = macro_head.block_number(),
            validity_start = validity_window_bn,
            "Starting a new validity synchronization process"
        );

        let next_election = Policy::election_block_after(validity_window_bn);

        // Now we determine which is the right root and block number to verify the first chunks
        let (verifier_block_number, expected_root, election_in_window) =
            if next_election < macro_head.block_number() {
                // This is the case where we are crossing an election block
                let election = self
                    .blockchain
                    .read()
                    .get_block_at(next_election, false)
                    .unwrap();

                (next_election, election.history_root().clone(), true)
            } else {
                // We don't have any election in between so we use the macro head
                (
                    macro_head.block_number(),
                    macro_head.header.history_root.clone(),
                    false,
                )
            };

        self.start_validity_chunk_request(
            peer_id,
            verifier_block_number,
            expected_root,
            validity_window_bn,
            election_in_window,
        );
    }

    pub fn poll_validity_window_chunks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(Ok((request, result, peer_id)))) =
            self.validity_queue.poll_next_unpin(cx)
        {
            log::trace!(peer=%peer_id, chunk_index= request.chunk_index, block_number =request.block_number,  "Processing response from validity queue");

            let macro_head = self.blockchain.read().macro_head();
            let macro_head_number = macro_head.block_number();
            let macro_history_root = macro_head.header.history_root;

            match result {
                Ok(chunk) => {
                    let peer_request = self.validity_requests.as_mut().unwrap();
                    let expected_root = peer_request.root_hash.clone();
                    let mut verifier_block_number = peer_request.verifier_block_number;

                    let leaf_index = peer_request.chunk_index * (CHUNK_SIZE as u32);
                    let chunk = chunk.chunk;

                    log::trace!(
                        leaf_index = leaf_index,
                        chunk_index = peer_request.chunk_index,
                        verifier_bn = verifier_block_number,
                        "Applying a new validity chunk"
                    );

                    // Verify the history chunk
                    let verification_result = chunk
                        .verify(&expected_root, leaf_index as usize)
                        .map_or(false, |result| result);

                    if verification_result {
                        let leaf_count = match &self.blockchain {
                            BlockchainProxy::Full(blockchain) => {
                                Blockchain::extend_validity_sync(
                                    blockchain.upgradable_read(),
                                    Policy::epoch_at(verifier_block_number),
                                    &chunk.history,
                                );

                                blockchain
                                    .read()
                                    .history_store
                                    .length_at(verifier_block_number, None)
                                    as usize
                            }
                            BlockchainProxy::Light(_) => unreachable!(),
                        };

                        // Get ready for requesting the next chunk
                        let mut chunk_index = peer_request.chunk_index + 1;

                        // We need to check if this is the last chunk:
                        let prover_mmr_size = chunk.proof.proof.mmr_size;

                        // Now we need to add the initial offseat
                        let total_size =
                            leaf_number_to_index(peer_request.initial_offset as usize + leaf_count);

                        if prover_mmr_size == total_size {
                            // We need to check if there was an election in between, if so, we need to proceed to the next epoch
                            if peer_request.election_in_window {
                                log::trace!(
                                    "Moving to the next epoch to continue syncing, current epoch {} ",
                                    Policy::epoch_at(verifier_block_number)
                                );

                                match &self.blockchain {
                                    BlockchainProxy::Full(blockchain) => {
                                        let root = blockchain
                                            .read()
                                            .history_store
                                            .get_history_tree_root(
                                                Policy::epoch_at(verifier_block_number),
                                                None,
                                            )
                                            .unwrap();
                                        assert_eq!(root, expected_root,"The final history root and the expected one, are not the same");
                                    }
                                    BlockchainProxy::Light(_) => todo!(),
                                }

                                // Move to the next epoch:
                                verifier_block_number = macro_head_number;
                                chunk_index = 0;
                                peer_request.initial_offset = 0;
                                peer_request.election_in_window = false;
                                peer_request.root_hash = macro_history_root.clone();
                                peer_request.verifier_block_number = verifier_block_number;
                            } else {
                                // No election in between, so we are done
                                log::debug!("Validity window syncing is complete");

                                match &self.blockchain {
                                    BlockchainProxy::Full(blockchain) => {
                                        let root = blockchain
                                            .read()
                                            .history_store
                                            .get_history_tree_root(
                                                Policy::epoch_at(macro_head_number),
                                                None,
                                            )
                                            .unwrap();

                                        assert_eq!(root, macro_history_root,"The final history root and the macro history root, are not the same");
                                    }
                                    BlockchainProxy::Light(_) => todo!(),
                                }

                                self.validity_queue.remove_peer(&peer_id);
                                self.syncing_peers.remove(&peer_id);

                                // We move all the peers from the sync queue to the synced peers.
                                for peer_id in self.syncing_peers.iter() {
                                    self.synced_validity_peers.push(*peer_id);
                                    self.validity_queue.remove_peer(peer_id);
                                }

                                // We are complete so we emit the peer
                                self.validity_requests = None;
                                self.syncing_peers.clear();

                                return Poll::Ready(Some(MacroSyncReturn::Good(peer_id)));
                            }
                        }

                        // Update the peer tracker structure
                        peer_request.chunk_index = chunk_index;

                        let request = RequestHistoryChunk {
                            epoch_number: Policy::epoch_at(verifier_block_number),
                            block_number: verifier_block_number,
                            chunk_index: chunk_index as u64,
                        };

                        log::trace!(
                            verifier_bn = verifier_block_number,
                            chunk_index = chunk_index,
                            "Adding a new validity window chunk request"
                        );

                        self.validity_queue.add_ids(vec![(request, None)]);
                    } else {
                        // If the chunk doesnt verify we disconnect from the peer
                        log::error!(peer=?peer_id,
                                    chunk=request.chunk_index,
                                    verifier_block=request.block_number,
                                    epoch=request.epoch_number,
                                    "The validity history chunk didn't verify, disconnecting from peer");

                        self.disconnect_peer(peer_id, CloseReason::MaliciousPeer)
                    }
                }
                Err(_err) => {
                    if request.epoch_number == 0 {
                        return Poll::Ready(Some(MacroSyncReturn::Good(peer_id)));
                    }
                    {
                        return Poll::Ready(Some(MacroSyncReturn::Outdated(peer_id)));
                    }
                }
            }
        }

        Poll::Pending
    }
}
