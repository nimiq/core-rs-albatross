use std::{
    sync::Arc,
    task::{Context, Poll},
};

use futures::{FutureExt, StreamExt};
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
use crate::messages::{
    HistoryChunk, RequestHistoryChunk, RequestValidityWindowStart, ValidityWindowStartResponse,
};
use crate::sync::{light::sync::ValidityChunkRequest, syncer::MacroSyncReturn};

#[cfg(feature = "full")]
impl<TNetwork: Network> LightMacroSync<TNetwork> {
    pub async fn discover_validity_window_items(
        network: Arc<TNetwork>,
        macro_head_number: u32,
        macro_head_hash: Blake2bHash,
        peer_id: TNetwork::PeerId,
    ) -> Result<ValidityWindowStartResponse, RequestError> {
        // Send the request to discover the start of the validity window.
        log::debug!("Requesting Validity Window Start items");
        network
            .request::<RequestValidityWindowStart>(
                RequestValidityWindowStart {
                    macro_head_number,
                    macro_head_hash,
                },
                peer_id,
            )
            .await
    }

    pub async fn request_validity_window_chunk(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        epoch_number: u32,
        block_number: u32,
        chunk_index: u64,
    ) -> Result<HistoryChunk, RequestError> {
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

    pub fn poll_validity_window_discover_requests(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some((request_result, peer_id))) =
            self.validity_window_start.poll_next_unpin(cx)
        {
            match request_result {
                Ok(response) => {
                    // We need to verify the validity window start proof
                    if let Some(proof) = response.proof {
                        let macro_head = self.blockchain.read().macro_head();

                        // Calculate the current validity window bn
                        let validity_window_bn = macro_head
                            .block_number()
                            .saturating_sub(Policy::transaction_validity_window_blocks());

                        // This must correspond to a macro block.
                        assert!(Policy::is_macro_block_at(validity_window_bn));

                        // Now we analize and check the transactions that we obtained from the proof:
                        // We verify there are only two txns in the proof
                        if proof.history.len() != 2 {
                            // There could be a special situation where the first transaction in the blockchain
                            // is located after the validity start.
                            // In this case we recieve a proof that contains only that transaction
                            if proof.history.len() == 1 && proof.positions[0] == 1 {
                                if proof.history[0].block_number < validity_window_bn {
                                    return Poll::Ready(Some(MacroSyncReturn::Outdated(peer_id)));
                                }

                                let verification_result = proof
                                    .verify(macro_head.header.history_root.clone())
                                    .map_or(false, |result| result);
                                if !verification_result {
                                    log::warn!(peer=%peer_id,"Validity start proof didnt verify, disconnecting peer");
                                    self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                    return Poll::Ready(None);
                                }

                                self.validity_requests.insert(
                                    peer_id,
                                    ValidityChunkRequest {
                                        verifier_block_number: macro_head.block_number(),
                                        root_hash: macro_head.header.history_root.clone(),
                                        chunk_index: 0,
                                        initial_offset: 0,
                                        validity_start: validity_window_bn,
                                    },
                                );

                                let network = Arc::clone(&self.network);

                                self.validity_window_chunks.push(
                                    async move {
                                        (
                                            Self::request_validity_window_chunk(
                                                network,
                                                peer_id,
                                                Policy::epoch_at(proof.history[0].block_number),
                                                macro_head.block_number(),
                                                0,
                                            )
                                            .await,
                                            peer_id,
                                        )
                                    }
                                    .boxed(),
                                );

                                return Poll::Pending;
                            } else {
                                log::warn!("Validity start proof doesn't contain two transactions");
                                self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                return Poll::Ready(None);
                            }
                        }

                        // Verify the txns leaf indexes are consecutive.
                        let positions = proof.positions.clone();
                        if positions[0] + 1 != positions[1] {
                            log::error!("The validity start transactions are not consecutive");
                            log::error!("Positions: {:?}", positions);
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // Now we extract the transactions from the proof
                        let txn_proof_bn;

                        // Extract the block numbers that correspond to the transactions
                        let first_txn = proof.history[0].block_number;
                        let second_txn = proof.history[1].block_number;

                        // They sould correspond to different block numbers:
                        // The second_txn in the proof should correspond to the first transaction in the validity start
                        // And the first_txn should correspond to the transaction before,
                        // Which should correspond to a previous block number (if the server is not lying and omitting txns)
                        if first_txn >= second_txn {
                            log::error!("Validity start transactions do not correspond to different block numbers");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // The txn that corresponds to a macro block is the one that is considered to belong to the validity window start
                        if Policy::is_macro_block_at(second_txn) {
                            txn_proof_bn = second_txn;
                        } else {
                            // This could be a special case where the validity start doesn't contain any txn
                            // In this case, the proof should contain two transactions:
                            // One before the validity start and one after the validity start.
                            // And there should be a macro block between the two (which is the validity start)
                            if Policy::macro_block_after(first_txn)
                                == Policy::macro_block_before(second_txn)
                            {
                                // This is the validity start
                                txn_proof_bn = Policy::macro_block_before(second_txn);
                            } else {
                                log::warn!(peer=%peer_id, "The validity start proof does not contain a valid format");
                                self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                return Poll::Ready(None);
                            }
                        }

                        if txn_proof_bn <= Policy::genesis_block_number() {
                            log::warn!(peer=%peer_id, "Received a validity start proof for the genesis block, this is malicious");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // We check if the proof corresponds to our validity window start
                        if txn_proof_bn != validity_window_bn {
                            if txn_proof_bn < validity_window_bn {
                                // Peer is outdated so we emit it.
                                return Poll::Ready(Some(MacroSyncReturn::Outdated(peer_id)));
                            } else {
                                log::warn!(peer=%peer_id,"The proof is ahead than our current validity window");
                                self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                return Poll::Ready(None);
                            }
                        }

                        let next_election = Policy::election_block_after(validity_window_bn);

                        // Now we determine which is the right root to verify the proof
                        let (verifier_block_number, expected_root) =
                            if Policy::is_election_block_at(validity_window_bn) {
                                // If the validity window starts at an election we use the start itself.
                                let election = self
                                    .blockchain
                                    .read()
                                    .get_block_at(validity_window_bn, false)
                                    .unwrap();
                                (validity_window_bn, election.history_root().clone())
                            } else if next_election < macro_head.block_number() {
                                // This is the case where we are crossing an election block
                                let election = self
                                    .blockchain
                                    .read()
                                    .get_block_at(next_election, false)
                                    .unwrap();
                                (next_election, election.history_root().clone())
                            } else {
                                // We don't have any election in between so we use the macro head
                                (
                                    macro_head.block_number(),
                                    macro_head.header.history_root.clone(),
                                )
                            };

                        let verification_result = proof
                            .verify(expected_root.clone())
                            .map_or(false, |result| result);

                        if !verification_result {
                            log::warn!(peer=%peer_id,"Validity start proof didnt verify, disconnecting peer");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // If the proof verified, we proceed to request the first history chunk
                        let epoch_number = Policy::epoch_at(validity_window_bn);

                        // We use the second transaction from the proof as the starting point, which:
                        // Corresponds to the validity start or the first transaction after the validity start
                        let first_leaf_index = proof.positions[1];

                        // Create a new chunk tracker structure for this peer
                        let first_chunk = (first_leaf_index as u32) / (CHUNK_SIZE as u32);

                        self.validity_requests.insert(
                            peer_id,
                            ValidityChunkRequest {
                                verifier_block_number,
                                root_hash: expected_root,
                                chunk_index: first_chunk,
                                initial_offset: first_chunk * CHUNK_SIZE as u32,
                                validity_start: validity_window_bn,
                            },
                        );

                        // Request the first chunk
                        self.validity_queue.add_peer(peer_id);

                        let request = RequestHistoryChunk {
                            epoch_number,
                            block_number: verifier_block_number,
                            chunk_index: first_chunk as u64,
                        };

                        self.validity_queue.add_ids(vec![(request, None)]);
                    } else {
                        // If no proof is provided, we need to check if the start of the validity window is the genesis
                        let macro_head = self.blockchain.read().macro_head();

                        let validity_window_bn = macro_head
                            .block_number()
                            .saturating_sub(Policy::transaction_validity_window_blocks());

                        if validity_window_bn <= Policy::genesis_block_number() {
                            // No inclusion proof, nor validity start for the genesis block
                            // So we just proceed to request the first history chunk.
                            let verifier_block_number = macro_head.block_number();

                            self.validity_requests.insert(
                                peer_id,
                                ValidityChunkRequest {
                                    verifier_block_number,
                                    root_hash: macro_head.header.history_root.clone(),
                                    chunk_index: 0,
                                    initial_offset: 0,
                                    validity_start: validity_window_bn,
                                },
                            );

                            self.validity_queue.add_peer(peer_id);

                            let request = RequestHistoryChunk {
                                epoch_number: Policy::epoch_at(macro_head.block_number()),
                                block_number: verifier_block_number,
                                chunk_index: 0 as u64,
                            };

                            self.validity_queue.add_ids(vec![(request, None)]);
                        } else {
                            log::error!(peer=?peer_id,"No validity start proof was provided for non genesis block");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }
                    }
                }
                Err(_) => todo!(),
            }
        }

        Poll::Pending
    }

    pub fn poll_validity_window_chunks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(Ok((_request, chunk, peer_id)))) =
            self.validity_queue.poll_next_unpin(cx)
        {
            log::debug!(" Processing response from validity queue");
            let synced_validity_start = self.synced_validity_start;

            let macro_head = self.blockchain.read().macro_head().block_number();
            let current_validity_start =
                macro_head.saturating_sub(Policy::transaction_validity_window_blocks());

            if synced_validity_start == current_validity_start {
                // Already synced
                return Poll::Ready(Some(MacroSyncReturn::Good(peer_id)));
            }

            if let Some(chunk) = chunk.chunk {
                let peer_request = self.validity_requests.get_mut(&peer_id).unwrap();
                let expected_root = &peer_request.root_hash;
                let verifier_block_number = peer_request.verifier_block_number;

                let leaf_index = peer_request.chunk_index * (CHUNK_SIZE as u32);

                // Verify the history chunk
                let verification_result = chunk
                    .verify(expected_root, leaf_index as usize)
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

                    // We need to check if this is the last chunk:
                    let prover_mmr_size = chunk.proof.proof.mmr_size;

                    // Now we need to add the initial offseat
                    let total_size =
                        leaf_number_to_index(peer_request.initial_offset as usize + leaf_count);

                    if prover_mmr_size == total_size {
                        log::debug!(perr=%peer_id, "Finished validity syncing with this peer");

                        // Signal the validity start that we are synced with.
                        self.synced_validity_start = peer_request.validity_start;

                        // We are complete so we emit the peer
                        self.remove_peer_requests(peer_id);

                        return Poll::Ready(Some(MacroSyncReturn::Good(peer_id)));
                    }

                    // Update the peer tracker structure
                    let chunk_index = peer_request.chunk_index + 1;
                    peer_request.chunk_index = chunk_index;

                    let request = RequestHistoryChunk {
                        epoch_number: Policy::epoch_at(verifier_block_number),
                        block_number: verifier_block_number,
                        chunk_index: chunk_index as u64,
                    };

                    self.validity_queue.add_ids(vec![(request, None)]);
                } else {
                    // If the chunk doesnt verify we disconnect from the peer
                    log::debug!("The chunk didn't verify, disconnecting from peer");
                    self.disconnect_peer(peer_id, CloseReason::MaliciousPeer)
                }
            } else {
                // Treating this as the peer doesnt have anything to sync
                return Poll::Ready(Some(MacroSyncReturn::Good(peer_id)));

                //If the peer didnt provide any History Chunk, we disconnect from the peer.
                //log::debug!(" The peer didn't provide any chunk, disconnecting from peer");
                //self.disconnect_peer(peer_id, CloseReason::MaliciousPeer)
            }
        }

        Poll::Pending
    }
}
