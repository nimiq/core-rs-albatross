use std::{
    sync::Arc,
    task::{Context, Poll},
};

use futures::StreamExt;
#[cfg(feature = "full")]
use nimiq_blockchain::{interface::HistoryInterface, Blockchain, CHUNK_SIZE};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    request::RequestError,
};
use nimiq_primitives::policy::Policy;

use super::LightMacroSync;
#[cfg(feature = "full")]
use crate::messages::{HistoryChunk, HistoryChunkError, RequestHistoryChunk};
use crate::sync::{light::sync::ValidityChunkRequest, syncer::MacroSyncReturn};

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
                root_hash: expected_root.clone(),
                chunk_index,
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
                expected_root = %expected_root,
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

        let validity_start = macro_head
            .block_number()
            .saturating_sub(Policy::transaction_validity_window_blocks());

        let validity_window_bn = if validity_start <= Policy::genesis_block_number() {
            Policy::genesis_block_number()
        } else {
            Policy::election_block_before(validity_start)
        };

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

    /// Process the history chunks that are received as part of the validity window synchronization process
    /// Each time a history chunk is received, it is verified and the history store is updated.
    pub fn poll_validity_window_chunks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(Ok((request, result, peer_id)))) =
            self.validity_queue.poll_next_unpin(cx)
        {
            log::trace!(peer=%peer_id, chunk_index=request.chunk_index, block_number=request.block_number,  "Processing response from validity queue");

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
                        let epoch_complete = match &self.blockchain {
                            BlockchainProxy::Full(blockchain) => {
                                let history_root = Blockchain::extend_validity_sync(
                                    blockchain.upgradable_read(),
                                    Policy::epoch_at(verifier_block_number),
                                    &chunk.history,
                                )
                                .unwrap();

                                history_root == expected_root
                            }
                            BlockchainProxy::Light(_) => unreachable!(),
                        };

                        // Get ready for requesting the next chunk
                        let mut chunk_index = peer_request.chunk_index + 1;

                        if epoch_complete {
                            // We need to check if there was an election in between, if so, we need to proceed to the next epoch
                            if peer_request.election_in_window {
                                log::trace!(
                                    current_epoch = Policy::epoch_at(verifier_block_number),
                                    new_verifier_bn = macro_head_number,
                                    new_expected_root = %macro_history_root,
                                    "Moving to the next epoch to continue syncing",
                                );

                                // Move to the next epoch:
                                verifier_block_number = macro_head_number;
                                chunk_index = 0;
                                peer_request.election_in_window = false;
                                peer_request.root_hash = macro_history_root.clone();
                                peer_request.verifier_block_number = verifier_block_number;
                            } else {
                                // No election in between, so we are done
                                log::debug!(
                                    synced_root = %expected_root,
                                    "Validity window syncing is complete"
                                );

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
                        // If the chunk doesn't verify we disconnect from the peer
                        log::error!(peer=?peer_id,
                                    chunk=request.chunk_index,
                                    verifier_block=request.block_number,
                                    epoch=request.epoch_number,
                                    "The validity history chunk didn't verify, disconnecting from peer");

                        // Remove the peer from the syncing process
                        self.validity_queue.remove_peer(&peer_id);
                        self.syncing_peers.remove(&peer_id);

                        // Disconnect and ban the peer
                        self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);

                        // Re add the request to the sync queue
                        self.validity_queue.add_ids(vec![(request, None)]);
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
