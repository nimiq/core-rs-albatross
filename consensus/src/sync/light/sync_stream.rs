use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt};
// `BlockError` from `nimiq_block` (block-level verification errors) is aliased to avoid clashing
// with `crate::messages::BlockError` (the request-response error used elsewhere in this file).
use nimiq_block::{Block, BlockError as BlockVerifyError};
#[cfg(feature = "full")]
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{CloseReason, Network, NetworkEvent};
#[cfg(feature = "full")]
use nimiq_primitives::policy::Policy;

use crate::{
    messages::BlockError,
    sync::{
        light::LightMacroSync,
        sync_interface::{MacroSync, MacroSyncReturn},
    },
};

/// Outcome of attempting to apply a single fetched macro/election block to the chain.
enum ApplyOutcome {
    /// The block was applied (Extended/Known/Ignored, or `previous_slots` updated for full nodes).
    Pushed,
    /// The block is below our current head; the peer that provided it is outdated.
    OutdatedPeer,
    /// The block is valid but does not extend our chain right now — its predecessor is missing (an
    /// earlier block in the sequence could not be fetched) or it is on a fork. The peer relayed a
    /// hash-matching block, so this is not misbehavior: skip it rather than banning the peer.
    Unfit,
    /// The block is intrinsically invalid; the responding peer must be banned.
    Ban,
}

/// Whether a failed macro-block push reflects the block not fitting our chain *right now* (a missing
/// or diverging predecessor) rather than the block being intrinsically invalid. In the former case
/// the peer that served a hash-matching block is not at fault and must not be banned.
fn is_unfitting_push_error(error: &PushError) -> bool {
    matches!(
        error,
        PushError::Orphan
            | PushError::InvalidSuccessor
            | PushError::InvalidPredecessor
            | PushError::InvalidFork
            | PushError::InvalidBlock(
                BlockVerifyError::InvalidParentElectionHash | BlockVerifyError::InvalidBlockNumber
            )
    )
}

impl<TNetwork: Network> LightMacroSync<TNetwork> {
    // This function is the one that starts the LightMacroSync process,
    // by adding peers into the MacroSync component.
    // It also removes peers from the internal data structures, when they leave
    fn poll_network_events(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer_id)) => {
                    // Remove the peer from internal data structures.
                    self.remove_peer(peer_id);
                }
                Ok(NetworkEvent::PeerJoined(peer_id, _)) => {
                    // Query if that peer provides the necessary services for syncing
                    if self.network.peer_provides_required_services(peer_id) {
                        // Start the macro sync process
                        self.add_peer(peer_id);
                    } else {
                        // We can't sync with this peer as it doesn't provide the services that we need.
                        // Emit the peer as incompatible.
                        return Poll::Ready(Some(MacroSyncReturn::Incompatible(peer_id)));
                    }
                }
                Ok(_) => {}
                Err(_) => return Poll::Ready(None),
            }
        }

        Poll::Pending
    }

    /// Polls for results from epoch ID requests made to peers.
    fn poll_epoch_ids(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(Some(epoch_ids))) = self.epoch_ids_stream.poll_next_unpin(cx) {
            // The peer might have disconnected during the request.
            if !self.network.has_peer(epoch_ids.sender) {
                continue;
            }

            // If the peer didn't find any of our locators, we are done with it and emit it.
            if !epoch_ids.locator_found {
                debug!(
                    peer_id = ?epoch_ids.sender,
                    "Peer is behind or on different chain"
                );
                return Poll::Ready(Some(MacroSyncReturn::Outdated(epoch_ids.sender)));
            } else if epoch_ids.ids.is_empty() && epoch_ids.checkpoint.is_none() {
                match self.blockchain {
                    #[cfg(feature = "full")]
                    BlockchainProxy::Full(ref blockchain) => {
                        // For the full sync, we need to make sure that the previous_slots have been set.
                        // If not, we need to request the corresponding macro block and set them.
                        let blockchain_rg = blockchain.read();
                        if blockchain_rg.state.previous_slots.is_none()
                            && blockchain_rg.election_head().block_number() > 0
                        {
                            let previous_election_block = blockchain_rg
                                .election_head()
                                .header
                                .parent_election_hash
                                .clone();
                            drop(blockchain_rg);

                            // Fetch the predecessor election block through the macro block queue and
                            // re-query this peer once it has been applied (`update_previous_slots`
                            // does not advance the head, so the tip-follow loop can't drive it).
                            if self
                                .in_flight_macro_blocks
                                .insert(previous_election_block.clone())
                            {
                                self.macro_block_queue.add_ids(vec![(
                                    previous_election_block.clone(),
                                    Some(epoch_ids.sender),
                                )]);
                            }
                            self.pending_followup_requests
                                .entry(previous_election_block)
                                .or_default()
                                .push(epoch_ids.sender);

                            continue;
                        }
                    }
                    _ => {}
                }
                // We are synced with this peer.
                debug!(
                    peer_id = ?epoch_ids.sender,
                    "Finished macro syncing with peer");

                if self.blockchain.read().can_enforce_validity_window() {
                    // We can enforce the validity window, so we are done.
                    return Poll::Ready(Some(MacroSyncReturn::Good(epoch_ids.sender)));
                } else {
                    #[cfg(feature = "full")]
                    self.start_validity_synchronization(epoch_ids.sender);
                }
            } else {
                #[cfg(feature = "full")]
                if let BlockchainProxy::Full(_) = self.blockchain {
                    let blockchain = self.blockchain.read();
                    let our_head = blockchain.block_number();
                    let can_enforce_validity_window = blockchain.can_enforce_validity_window();
                    drop(blockchain);

                    // Calculate an upper bound on the peer's head block.
                    // The upper bound is the last micro block of the latest batch of the peer.
                    let peer_head_upper_bound = epoch_ids
                        .checkpoint
                        .as_ref()
                        .map(|checkpoint| checkpoint.block_number)
                        .unwrap_or_else(|| {
                            if epoch_ids.checkpoint_epoch_number() == 0 {
                                return 0;
                            }
                            Policy::first_block_of(epoch_ids.checkpoint_epoch_number() as u32)
                                .unwrap_or(u32::MAX)
                        })
                        .saturating_add(Policy::blocks_per_batch() - 1);

                    if peer_head_upper_bound.saturating_sub(our_head) <= self.full_sync_threshold {
                        if can_enforce_validity_window {
                            log::debug!(
                                our_head,
                                peer_head = peer_head_upper_bound,
                                peer_id = %epoch_ids.sender,
                                "Peer is sufficiently close for a live sync instead of macro sync."
                            );
                            return Poll::Ready(Some(MacroSyncReturn::Good(epoch_ids.sender)));
                        } else {
                            #[cfg(feature = "full")]
                            self.start_validity_synchronization(epoch_ids.sender);
                        }
                    }
                }
            }

            // Request the macro headers from the peer or emit it accordingly
            if let Some(macro_sync_return) = self.request_macro_headers(epoch_ids) {
                match macro_sync_return {
                    MacroSyncReturn::Good(peer_id) => {
                        // We are synced with this peer.
                        debug!(?peer_id, "Finished macro syncing with peer");

                        if self.blockchain.read().can_enforce_validity_window() {
                            // We can enforce the validity window, so we are done.
                            return Poll::Ready(Some(MacroSyncReturn::Good(peer_id)));
                        } else {
                            #[cfg(feature = "full")]
                            self.start_validity_synchronization(peer_id);
                        }
                    }
                    _ => {
                        // Propagate any other sync result (outdated is the only one that can be emitted by the macro headers process)
                        return Poll::Ready(Some(macro_sync_return));
                    }
                }
            }
        }

        Poll::Pending
    }

    /// Applies a single fetched macro/election block to the chain and returns the outcome. Used by
    /// `poll_macro_block_queue` for every macro block (seed, forward catch-up, `previous_slots`).
    /// Does not disconnect peers or touch request bookkeeping — the caller acts on the outcome.
    fn apply_macro_block(&self, block: Block) -> ApplyOutcome {
        // Check if the block is still valid for us or if it is outdated before trying to apply it
        let push_result = match self.blockchain {
            #[cfg(feature = "full")]
            BlockchainProxy::Full(ref full_blockchain) => {
                let blockchain = full_blockchain.upgradable_read();

                let latest_block_number = blockchain.block_number();
                // Get the block number of the election block before the current election block.
                let current_election_block_number = blockchain.election_head().block_number();
                let previous_election_block_number = if current_election_block_number > 0 {
                    Some(Policy::election_block_before(current_election_block_number))
                } else {
                    None
                };

                // If the block matches the previous election block, we use it to update the
                // `previous_slots`. Otherwise, check if it's outdated / push the macro block.
                if previous_election_block_number == Some(block.block_number()) {
                    Blockchain::update_previous_slots(blockchain, block.clone())
                } else if block.block_number() < latest_block_number {
                    return ApplyOutcome::OutdatedPeer;
                } else {
                    Blockchain::push_macro(blockchain, block.clone())
                }
            }
            BlockchainProxy::Light(ref light_blockchain) => {
                let blockchain = light_blockchain.upgradable_read();

                let latest_block_number = blockchain.block_number();

                // Bootstrap checkpoint (pico sync fallback): the checkpoint election block has no
                // on-chain predecessor yet, so it must be seeded via push_pico_election rather than
                // push_macro (which would fail macro-successor verification). The response hash was
                // already verified against the requested checkpoint hash, so matching the block
                // number identifies the seed block. push_pico_election is idempotent (returns
                // Known/Ignored) if another peer already seeded the chain.
                if let Some(checkpoint) = &self.bootstrap_checkpoint
                    && blockchain.block_number() < checkpoint.block_number
                    && block.is_election()
                    && block.block_number() == checkpoint.block_number
                {
                    LightBlockchain::push_pico_election(blockchain, block.clone())
                } else if block.block_number() < latest_block_number {
                    return ApplyOutcome::OutdatedPeer;
                } else {
                    LightBlockchain::push_macro(blockchain, block.clone())
                }
            }
        };

        match push_result {
            Ok(push_result) => {
                log::debug!(
                    block_number = block.block_number(),
                    ?push_result,
                    "Pushed a macro block",
                );
                ApplyOutcome::Pushed
            }
            // The block doesn't extend our chain (missing predecessor or fork). The serving peer is
            // not at fault — skip the block instead of banning it.
            Err(error) if is_unfitting_push_error(&error) => {
                log::debug!(
                    block_number = block.block_number(),
                    ?error,
                    "Skipping macro block that does not extend our chain",
                );
                ApplyOutcome::Unfit
            }
            Err(error) => {
                log::warn!(
                    block_number = block.block_number(),
                    ?error,
                    "Failed to push macro block",
                );
                ApplyOutcome::Ban
            }
        }
    }

    /// Polls the bounded, failover macro block queue (the checkpoint seed, the forward catch-up, and
    /// the full-node `previous_slots` recovery block all flow through here). Applies fetched blocks
    /// in request order; a misbehaving peer is banned while others keep fetching via failover, and a
    /// fully-exhausted block is logged and left to be re-requested later (no wedge). After draining,
    /// it advances the tip-follow loop (re-querying peers whose reported target our head has reached,
    /// for seed + forward) and resolves any `previous_slots` follow-ups. Never emits a
    /// `MacroSyncReturn` itself (always `Poll::Pending`); peer transitions happen via re-queried
    /// epoch ids or `disconnect_peer`.
    fn poll_macro_block_queue(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        // Set when a block exhausts its retries across all peers. The successors of an exhausted
        // block may still be in flight (`PENDING_SIZE` ahead), so we cannot decide recovery in the
        // `Err` arm itself — those successors will resolve and be skipped as `Unfit` (missing
        // predecessor), emptying the queue without ever advancing the head. We therefore recover
        // once, after the drain loop, if an exhaustion left the queue empty.
        let mut fetch_exhausted = false;
        while let Poll::Ready(Some(result)) = self.macro_block_queue.poll_next_unpin(cx) {
            match result {
                Ok((hash, Ok(block), peer_id)) => {
                    self.in_flight_macro_blocks.remove(&hash);

                    if !block.is_macro() {
                        log::warn!(%peer_id, "Banning peer due to a non macro block response");
                        self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                        self.resolve_followups(&hash);
                        continue;
                    }

                    match self.apply_macro_block(block) {
                        // Applied normally; a backward `previous_slots` block that went stale because
                        // a forward push already advanced the head (and set previous_slots); or a
                        // block that doesn't fit our chain (missing predecessor / fork). None of
                        // these is peer misbehavior — re-query the follow-up peers to finish them.
                        ApplyOutcome::Pushed | ApplyOutcome::OutdatedPeer | ApplyOutcome::Unfit => {
                            self.resolve_followups(&hash);
                        }
                        ApplyOutcome::Ban => {
                            log::warn!(%peer_id, "Banning peer that served an invalid macro block");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            self.resolve_followups(&hash);
                        }
                    }
                }
                // A hash mismatch is the responder's fault (it returned a block other than the one
                // requested) — ban it. The request function only ever surfaces this block error;
                // every other remote block error is retried via failover (see `MacroBlockError`),
                // so an honest peer that merely lacks the block is not disconnected.
                Ok((hash, Err(BlockError::ResponseHashMismatch), peer_id)) => {
                    self.in_flight_macro_blocks.remove(&hash);
                    log::warn!(%peer_id, "Banning peer that replied with a block other than the requested one");
                    self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                    self.resolve_followups(&hash);
                }
                // Unreachable in practice (the request function only surfaces `ResponseHashMismatch`),
                // but handle defensively without disconnecting the peer.
                Ok((hash, Err(error), peer_id)) => {
                    self.in_flight_macro_blocks.remove(&hash);
                    trace!(%error, %peer_id, "Unexpected remote block error; skipping");
                    self.resolve_followups(&hash);
                }
                // All retry attempts across all peers were exhausted for this block. Don't ban or
                // disconnect (honest peers may simply not have it yet); drop it from the in-flight
                // set so it can be re-requested. Recovery is deferred to after the drain loop (see
                // `fetch_exhausted`): the successors already in flight will resolve and be skipped
                // as `Unfit`, so deciding here would miss the mid-sequence case.
                Err(hash) => {
                    self.in_flight_macro_blocks.remove(&hash);
                    log::warn!(%hash, "Could not fetch macro block from any peer after retries");
                    self.resolve_followups(&hash);
                    fetch_exhausted = true;
                }
            }
        }

        // Tip-follow loop: re-query peers whose reported target our head has now reached. They are
        // then emitted `Good` ("nothing new") or report the moving tip and enqueue the next epochs.
        let head = self.blockchain.read().block_number();
        let satisfied: Vec<_> = self
            .waiting_macro_peers
            .iter()
            .filter(|&(_, &target)| target <= head)
            .map(|(peer_id, _)| *peer_id)
            .collect();
        for peer_id in satisfied {
            self.waiting_macro_peers.remove(&peer_id);
            if self.network.has_peer(peer_id) {
                self.request_epoch_ids_from(peer_id);
            }
        }

        // Recover from an exhausted fetch that left us unable to advance: a block was exhausted and
        // the queue is now empty (its successors, if any, resolved and were skipped as `Unfit`).
        // The peers still in `waiting_macro_peers` are blocked on a head that will never arrive on
        // its own, so re-add them (re-seeding or re-requesting epoch ids as appropriate for our
        // current head) to recover instead of wedging until a new peer joins. Gating on
        // `fetch_exhausted` (rather than emptiness alone) keeps a misconfigured checkpoint — which
        // only ever skips via `Unfit`, never `Err` — silently failing to seed rather than looping.
        if fetch_exhausted && self.macro_block_queue.is_empty() {
            let waiting: Vec<_> = self.waiting_macro_peers.drain().map(|(p, _)| p).collect();
            for peer_id in waiting {
                if self.network.has_peer(peer_id) {
                    self.add_peer(peer_id);
                }
            }
        }

        Poll::Pending
    }

    /// Re-queries the `previous_slots` follow-up peers registered for a just-finished queued block
    /// (no-op for forward/seed blocks, which never register one). Whatever the outcome — applied,
    /// stale, unfitting, or unfetchable — the waiting peer is re-queried for epoch ids: it is then
    /// emitted `Good` once `previous_slots` is set, `Outdated` if it is now behind, or it re-requests
    /// the block. An honest peer is never disconnected for a fetch we could not complete.
    fn resolve_followups(&mut self, hash: &Blake2bHash) {
        let Some(peers) = self.pending_followup_requests.remove(hash) else {
            return;
        };
        for peer_id in peers {
            if self.network.has_peer(peer_id) {
                self.request_epoch_ids_from(peer_id);
            }
        }
    }
}

impl<TNetwork: Network> Stream for LightMacroSync<TNetwork> {
    type Item = MacroSyncReturn<TNetwork::PeerId>;

    /// Polls all syncing stages in order and returns the result of the first completed stage.
    ///
    /// This function implements the `Stream` trait for `LightMacroSync`, making it a driver that
    /// progresses syncing in steps. It checks for:
    /// - Peer join/leave events
    /// - Epoch ID responses
    /// - Macro block headers (seed, forward catch-up, previous_slots)
    /// - Validity window chunks
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(o) = self.poll_network_events(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_epoch_ids(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_macro_block_queue(cx) {
            return Poll::Ready(o);
        }
        #[cfg(feature = "full")]
        {
            if let Poll::Ready(o) = self.poll_validity_window_chunks(cx) {
                return Poll::Ready(o);
            }

            if let Some(peer_id) = self.synced_validity_peers.pop() {
                // We move the validity synced peer to another round of macro sync,
                // to re-check if there is new state that we need to sync with.
                self.add_peer(peer_id);
            }
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::StreamExt;
    use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
    use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
    use nimiq_blockchain_proxy::BlockchainProxy;
    use nimiq_database::{mdbx::MdbxDatabase, traits::WriteTransaction};
    use nimiq_genesis::HardcodedElection;
    use nimiq_light_blockchain::LightBlockchain;
    use nimiq_network_interface::{
        network::Network,
        request::{request_handler, Handle},
    };
    use nimiq_network_mock::{MockHub, MockNetwork};
    use nimiq_primitives::{networks::NetworkId, policy::Policy};
    use nimiq_test_log::test;
    use nimiq_test_utils::blockchain::{produce_macro_blocks_with_txns, signing_key, voting_key};
    use nimiq_utils::{spawn, time::OffsetTime};
    use parking_lot::RwLock;

    use crate::{
        messages::{BlockError, RequestBlock, RequestHistoryChunk, RequestMacroChain},
        sync::{light::LightMacroSync, sync_interface::MacroSyncReturn},
    };

    fn blockchain() -> BlockchainProxy {
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        BlockchainProxy::Full(Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        )))
    }

    fn light_blockchain() -> BlockchainProxy {
        BlockchainProxy::Light(Arc::new(RwLock::new(LightBlockchain::new(
            NetworkId::UnitAlbatross,
        ))))
    }

    fn spawn_request_handlers<TNetwork: Network>(
        network: &Arc<TNetwork>,
        blockchain: &BlockchainProxy,
    ) {
        spawn(request_handler(
            network,
            network.receive_requests::<RequestMacroChain>(),
            blockchain,
        ));

        spawn(request_handler(
            network,
            network.receive_requests::<RequestBlock>(),
            blockchain,
        ));

        match blockchain {
            BlockchainProxy::Full(full_blockchain) => {
                spawn(request_handler(
                    network,
                    network.receive_requests::<RequestHistoryChunk>(),
                    full_blockchain,
                ));
            }
            BlockchainProxy::Light(_) => {}
        };
    }

    #[test(tokio::test)]
    async fn it_terminates_if_there_is_nothing_to_sync() {
        async fn test(chain1: BlockchainProxy, chain2: BlockchainProxy) {
            let mut hub = MockHub::default();
            let net1 = Arc::new(hub.new_network());
            let net2 = Arc::new(hub.new_network());

            let mut sync = LightMacroSync::<MockNetwork>::new(
                chain1.clone(),
                Arc::clone(&net1),
                net1.subscribe_events(),
                0,
                None,
            );

            spawn_request_handlers(&net2, &chain2.clone());
            net1.dial_mock(&net2);

            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => {
                    assert_eq!(chain1.read().head(), chain2.read().head());
                }
                res => panic!("Unexpected HistorySyncReturn: {res:?}"),
            }
        }

        test(light_blockchain(), light_blockchain()).await;
        test(blockchain(), light_blockchain()).await;
        test(light_blockchain(), blockchain()).await;
        test(blockchain(), blockchain()).await;
    }

    // Tries to sync to a full blockchain.
    #[test(tokio::test)]
    async fn it_can_sync_a_single_finalized_epoch() {
        async fn test(chain1: BlockchainProxy) {
            let mut hub = MockHub::default();
            let net1 = Arc::new(hub.new_network());
            let net2 = Arc::new(hub.new_network());

            let chain2 = blockchain();

            let producer = BlockProducer::new(signing_key(), voting_key());
            if let BlockchainProxy::Full(ref chain2) = chain2 {
                produce_macro_blocks_with_txns(
                    &producer,
                    chain2,
                    Policy::batches_per_epoch() as usize,
                    1,
                    0,
                );
            }
            assert_eq!(
                chain2.read().block_number(),
                Policy::blocks_per_epoch() + Policy::genesis_block_number()
            );

            let mut sync = LightMacroSync::<MockNetwork>::new(
                chain1.clone(),
                Arc::clone(&net1),
                net1.subscribe_events(),
                0,
                None,
            );

            spawn_request_handlers(&net2, &chain2.clone());
            net1.dial_mock(&net2);

            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => {
                    assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
                }
                res => panic!("Unexpected HistorySyncReturn: {res:?}"),
            }
        }

        test(light_blockchain()).await;
        test(blockchain()).await;
    }

    #[test(tokio::test)]
    async fn it_can_sync_a_single_finalized_epoch_and_batch() {
        async fn test(chain1: BlockchainProxy) {
            let mut hub = MockHub::default();
            let net1 = Arc::new(hub.new_network());
            let net2 = Arc::new(hub.new_network());

            let chain2 = blockchain();

            let producer = BlockProducer::new(signing_key(), voting_key());
            if let BlockchainProxy::Full(ref chain2) = chain2 {
                produce_macro_blocks_with_txns(
                    &producer,
                    chain2,
                    (Policy::batches_per_epoch() + 1) as usize,
                    1,
                    0,
                );
            }
            assert_eq!(
                chain2.read().block_number(),
                Policy::blocks_per_epoch()
                    + Policy::blocks_per_batch()
                    + Policy::genesis_block_number()
            );

            let mut sync = LightMacroSync::<MockNetwork>::new(
                chain1.clone(),
                Arc::clone(&net1),
                net1.subscribe_events(),
                0,
                None,
            );

            spawn_request_handlers(&net2, &chain2.clone());
            net1.dial_mock(&net2);

            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => {
                    assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
                }
                res => panic!("Unexpected HistorySyncReturn: {res:?}"),
            }
        }

        test(light_blockchain()).await;
        test(blockchain()).await;
    }

    // Tries to sync to a full blockchain.
    #[test(tokio::test)]
    async fn it_can_sync_multiple_batches() {
        async fn test(chain1: BlockchainProxy) {
            let mut hub = MockHub::default();
            let net1 = Arc::new(hub.new_network());
            let net2 = Arc::new(hub.new_network());

            let chain2 = blockchain();
            let num_batches = (Policy::batches_per_epoch() - 1) as usize;
            let producer = BlockProducer::new(signing_key(), voting_key());
            if let BlockchainProxy::Full(ref chain2) = chain2 {
                produce_macro_blocks_with_txns(&producer, chain2, num_batches, 1, 0);
            }
            assert_eq!(
                chain2.read().block_number(),
                num_batches as u32 * Policy::blocks_per_batch() + Policy::genesis_block_number()
            );

            let mut sync = LightMacroSync::<MockNetwork>::new(
                chain1.clone(),
                Arc::clone(&net1),
                net1.subscribe_events(),
                0,
                None,
            );

            spawn_request_handlers(&net2, &chain2.clone());
            net1.dial_mock(&net2);

            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => {
                    assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
                }
                res => panic!("Unexpected HistorySyncReturn: {res:?}"),
            }
        }

        test(light_blockchain()).await;
        test(blockchain()).await;
    }

    #[test(tokio::test)]
    async fn it_fetches_dangling_macro_block() {
        async fn test(num_extra_epochs: u32) {
            let chain1 = blockchain();
            let mut hub = MockHub::default();
            let net1 = Arc::new(hub.new_network());
            let net2 = Arc::new(hub.new_network());

            let chain2 = blockchain();

            let producer = BlockProducer::new(signing_key(), voting_key());
            if let BlockchainProxy::Full(ref chain2) = chain2 {
                produce_macro_blocks_with_txns(
                    &producer,
                    chain2,
                    Policy::batches_per_epoch() as usize * (num_extra_epochs + 2) as usize,
                    1,
                    0,
                );
            }
            assert_eq!(
                chain2.read().block_number(),
                Policy::blocks_per_epoch() * (num_extra_epochs + 2)
                    + Policy::genesis_block_number()
            );
            if let BlockchainProxy::Full(ref chain1) = chain1 {
                let block_to_delete = chain2
                    .read()
                    .get_block_at(
                        Policy::blocks_per_epoch() + Policy::genesis_block_number(),
                        true,
                    )
                    .unwrap();

                assert_eq!(
                    Blockchain::push_macro(chain1.upgradable_read(), block_to_delete.clone()),
                    Ok(PushResult::Extended),
                );
                assert_eq!(
                    Blockchain::push_macro(
                        chain1.upgradable_read(),
                        chain2
                            .read()
                            .get_block_at(
                                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                                true
                            )
                            .unwrap(),
                    ),
                    Ok(PushResult::Extended),
                );

                let mut chain1_wg = chain1.write();
                let mut txn = chain1_wg.write_transaction();
                chain1_wg.chain_store.remove_chain_info(
                    &mut txn,
                    &block_to_delete.hash(),
                    block_to_delete.block_number(),
                );
                txn.commit();
                chain1_wg.state.previous_slots = None;
            }

            let mut sync = LightMacroSync::<MockNetwork>::new(
                chain1.clone(),
                Arc::clone(&net1),
                net1.subscribe_events(),
                0,
                None,
            );

            spawn_request_handlers(&net2, &chain2.clone());
            net1.dial_mock(&net2);

            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => {
                    assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
                }
                res => panic!("Unexpected HistorySyncReturn: {res:?}"),
            }

            assert_eq!(
                chain1.read().previous_validators(),
                chain2.read().previous_validators()
            );
        }

        test(0).await;
        test(1).await;
    }

    /// A pico/light client configured with a hardcoded checkpoint bootstraps by fetching the
    /// checkpoint election block (by hash), seeding the chain to it via `push_pico_election`
    /// (its predecessor is never on-chain), and then syncing forward from there.
    ///
    /// The checkpoint is the *epoch-2* election block, so the epoch-1 election block (its
    /// predecessor) is never on the fresh client's chain. That is what makes the seed branch
    /// load-bearing: a plain `push_macro` would fail macro-successor verification.
    #[test(tokio::test)]
    async fn it_bootstraps_from_checkpoint() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Server: a full chain spanning three full epochs, so its head is the epoch-3 election
        // block and the epoch-2 election block (the checkpoint) has an on-chain predecessor the
        // client will never fetch.
        let chain2 = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain2) = chain2 {
            produce_macro_blocks_with_txns(
                &producer,
                chain2,
                Policy::batches_per_epoch() as usize * 3,
                1,
                0,
            );
        }
        assert_eq!(
            chain2.read().block_number(),
            Policy::blocks_per_epoch() * 3 + Policy::genesis_block_number()
        );

        // The checkpoint points at the epoch-2 election block: a fresh client must seed to it
        // (its epoch-1 predecessor is not on-chain) and then sync forward to the epoch-3 head.
        let cp_block = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let checkpoint = HardcodedElection {
            block_number: cp_block.block_number(),
            hash: cp_block.hash(),
        };

        // Client: fresh light chain at genesis, configured to bootstrap from the checkpoint.
        let chain1 = light_blockchain();
        assert!(chain1.read().block_number() < checkpoint.block_number);
        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            Some(checkpoint),
        );

        spawn_request_handlers(&net2, &chain2.clone());
        net1.dial_mock(&net2);

        // On a fresh genesis chain `add_peer` requests *only* the checkpoint block (epoch-2),
        // whose predecessor (epoch-1) is not on-chain. Without the seed branch in
        // `apply_macro_block`, `push_macro` would reject it (failed macro-successor verification)
        // and ban the peer, so no `Good` would ever be emitted. Reaching the epoch-3 head also
        // proves forward sync continues after the seed.
        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().block_number(), chain2.read().block_number());
                assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
            }
            res => panic!("Unexpected MacroSyncReturn: {res:?}"),
        }
    }

    /// When several peers are available before the chain is seeded, the checkpoint block is
    /// requested from each of them. The first response seeds the chain; a later response for the
    /// same (now on-chain or behind) block must be handled gracefully (`push_pico_election` /
    /// `push_macro` return `Known`/`Ignored`, or the peer is emitted as `Outdated`) and never
    /// treated as a malicious block.
    #[test(tokio::test)]
    async fn it_bootstraps_from_checkpoint_with_multiple_peers() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());

        // Two server endpoints serving the same three-epoch chain; checkpoint = epoch-2 election.
        let chain2 = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain2) = chain2 {
            produce_macro_blocks_with_txns(
                &producer,
                chain2,
                Policy::batches_per_epoch() as usize * 3,
                1,
                0,
            );
        }

        let cp_block = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let checkpoint = HardcodedElection {
            block_number: cp_block.block_number(),
            hash: cp_block.hash(),
        };

        let chain1 = light_blockchain();
        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            Some(checkpoint),
        );

        spawn_request_handlers(&net2, &chain2.clone());
        spawn_request_handlers(&net3, &chain2.clone());
        net1.dial_mock(&net2);
        net1.dial_mock(&net3);

        // Drive until a peer reports a successful (`Good`) sync. A second peer whose checkpoint
        // block is now behind the seeded chain may legitimately be emitted as `Outdated`; that is
        // fine. A wrongly-banned peer would instead terminate the stream with `None` and fail here.
        loop {
            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => break,
                Some(MacroSyncReturn::Outdated(_)) => continue,
                res => panic!("Unexpected MacroSyncReturn: {res:?}"),
            }
        }
        assert_eq!(chain1.read().block_number(), chain2.read().block_number());
        assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
    }

    /// Forward catch-up after the checkpoint spans many epochs (more than the queue's
    /// `PENDING_SIZE` window), so the election blocks are fetched in bounded, ordered batches
    /// through `macro_block_queue` and applied in epoch order until the head is reached.
    #[test(tokio::test)]
    async fn it_syncs_many_epochs_from_checkpoint_via_queue() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Server: eight full epochs (head = epoch-8 election block).
        let chain2 = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain2) = chain2 {
            produce_macro_blocks_with_txns(
                &producer,
                chain2,
                Policy::batches_per_epoch() as usize * 8,
                1,
                0,
            );
        }

        // Checkpoint = epoch-2 election block; forward sync then covers epochs 3..=8, i.e. six
        // election blocks, more than `PENDING_SIZE` (5) in flight at once.
        let cp_block = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let checkpoint = HardcodedElection {
            block_number: cp_block.block_number(),
            hash: cp_block.hash(),
        };

        let chain1 = light_blockchain();
        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            Some(checkpoint),
        );

        spawn_request_handlers(&net2, &chain2.clone());
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().block_number(), chain2.read().block_number());
                assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
            }
            res => panic!("Unexpected MacroSyncReturn: {res:?}"),
        }
    }

    /// Several peers report the same multi-epoch chain. The forward election blocks must be fetched
    /// once (deduplicated across peers via `in_flight_macro_blocks`), applied in epoch order, and
    /// every peer eventually resolved (`Good`/`Outdated`) without any being wrongly banned (which
    /// would terminate the stream with `None`).
    #[test(tokio::test)]
    async fn it_syncs_many_epochs_from_checkpoint_with_multiple_peers() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());

        let chain2 = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain2) = chain2 {
            produce_macro_blocks_with_txns(
                &producer,
                chain2,
                Policy::batches_per_epoch() as usize * 8,
                1,
                0,
            );
        }

        let cp_block = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let checkpoint = HardcodedElection {
            block_number: cp_block.block_number(),
            hash: cp_block.hash(),
        };

        let chain1 = light_blockchain();
        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            Some(checkpoint),
        );

        spawn_request_handlers(&net2, &chain2.clone());
        spawn_request_handlers(&net3, &chain2.clone());
        net1.dial_mock(&net2);
        net1.dial_mock(&net3);

        // A peer whose checkpoint/epochs are behind the seeded chain may legitimately be emitted as
        // `Outdated`; a wrongly-banned peer would instead terminate the stream with `None`.
        loop {
            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => break,
                Some(MacroSyncReturn::Outdated(_)) => continue,
                res => panic!("Unexpected MacroSyncReturn: {res:?}"),
            }
        }
        assert_eq!(chain1.read().block_number(), chain2.read().block_number());
        assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
    }

    /// A peer that answers epoch ids but cannot serve block requests must not stall or fail the
    /// sync: the bounded queue fails each block over to another peer. Here `net3` only handles
    /// `RequestMacroChain` (no `RequestBlock`), so blocks preferred to it are re-requested from the
    /// fully-serving `net2`, and the sync still reaches the head.
    #[test(tokio::test)]
    async fn it_fails_over_when_a_peer_serves_no_blocks() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());

        let chain2 = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain2) = chain2 {
            produce_macro_blocks_with_txns(
                &producer,
                chain2,
                Policy::batches_per_epoch() as usize * 5,
                1,
                0,
            );
        }

        let cp_block = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let checkpoint = HardcodedElection {
            block_number: cp_block.block_number(),
            hash: cp_block.hash(),
        };

        let chain1 = light_blockchain();
        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            Some(checkpoint),
        );

        // net2 serves everything; net3 answers only epoch ids (no `RequestBlock` handler), so block
        // requests preferred to net3 fail fast and must fail over to net2. Dial net3 first so its
        // epoch ids tend to arrive first and it is preferred for the (failing) block requests.
        spawn_request_handlers(&net2, &chain2.clone());
        spawn(request_handler(
            &net3,
            net3.receive_requests::<RequestMacroChain>(),
            &chain2.clone(),
        ));
        net1.dial_mock(&net3);
        net1.dial_mock(&net2);

        loop {
            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) => break,
                Some(MacroSyncReturn::Outdated(_)) => continue,
                res => panic!("Unexpected MacroSyncReturn: {res:?}"),
            }
        }
        assert_eq!(chain1.read().block_number(), chain2.read().block_number());
        assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
    }

    /// Mid-sequence fetch exhaustion during forward sync must not wedge the client. A single peer
    /// withholds one mid-sequence election block (epoch 5) for the first `max_tries` requests, so it
    /// exhausts its retries while its successors (epochs 6..=8) are already in flight. Those
    /// successors then resolve and are skipped as `Unfit` (their predecessor, epoch 5, never
    /// applied), draining the queue without advancing the head — the queue is *not* empty at the
    /// moment exhaustion fires, so the recovery in the `Err` arm alone would miss it. The post-drain
    /// recovery must re-add the waiting peer, whose retry now serves epoch 5, letting the sync reach
    /// the head. Without that recovery this test hangs.
    #[test(tokio::test)]
    async fn it_recovers_from_mid_sequence_exhaustion() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Server: eight full epochs (head = epoch-8 election block).
        let chain2 = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain2) = chain2 {
            produce_macro_blocks_with_txns(
                &producer,
                chain2,
                Policy::batches_per_epoch() as usize * 8,
                1,
                0,
            );
        }

        // Checkpoint = epoch-2 election block; forward sync then covers epochs 3..=8.
        let cp_block = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let checkpoint = HardcodedElection {
            block_number: cp_block.block_number(),
            hash: cp_block.hash(),
        };

        // The mid-sequence block we withhold: the epoch-5 election block.
        let withheld_hash = chain2
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 5 + Policy::genesis_block_number(),
                false,
            )
            .unwrap()
            .hash();

        let chain1 = light_blockchain();
        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            Some(checkpoint),
        );

        // net2 serves epoch ids normally, but its custom `RequestBlock` handler answers
        // `TargetHashNotFound` for the withheld block until it has been requested `max_tries` times
        // (4 for a single peer), then serves it like any other block.
        spawn(request_handler(
            &net2,
            net2.receive_requests::<RequestMacroChain>(),
            &chain2.clone(),
        ));
        let chain_for_blocks = chain2.clone();
        let net2_blocks = Arc::clone(&net2);
        spawn(async move {
            let mut requests = net2_blocks.receive_requests::<RequestBlock>();
            let mut withheld_attempts = 0u32;
            while let Some((msg, request_id, peer_id)) = requests.next().await {
                let response = if msg.hash == withheld_hash && withheld_attempts < 4 {
                    withheld_attempts += 1;
                    Err(BlockError::TargetHashNotFound)
                } else {
                    Handle::<MockNetwork, _>::handle(&msg, peer_id, &chain_for_blocks)
                };
                net2_blocks
                    .respond::<RequestBlock>(request_id, response)
                    .await
                    .ok();
            }
        });
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().block_number(), chain2.read().block_number());
                assert_eq!(chain1.read().head_hash(), chain2.read().head_hash());
            }
            res => panic!("Unexpected MacroSyncReturn: {res:?}"),
        }
    }

    /// A full node with `previous_slots = None` connects to two peers at once: a shorter one at the
    /// node's own head (empty epoch ids → triggers the backward `previous_slots` fetch) and a longer
    /// one (→ forward fetch). Both fetches share `macro_block_queue`. Regardless of which resolves
    /// first, the node must reach the longer head with correct `previous_slots` and emit `Good` —
    /// exercising the previous_slots / forward interleaving + the follow-up re-query.
    #[test(tokio::test)]
    async fn it_sets_previous_slots_while_forward_syncing() {
        let chain1 = blockchain();
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net_short = Arc::new(hub.new_network());
        let net_long = Arc::new(hub.new_network());

        // Long server: three full epochs (head = epoch-3 election block).
        let chain_long = blockchain();
        let producer = BlockProducer::new(signing_key(), voting_key());
        if let BlockchainProxy::Full(ref chain_long) = chain_long {
            produce_macro_blocks_with_txns(
                &producer,
                chain_long,
                Policy::batches_per_epoch() as usize * 3,
                1,
                0,
            );
        }

        // The epoch-1 and epoch-2 election blocks, reused to build the short server and the client
        // so their election blocks are byte-identical to the long server's.
        let epoch1 = chain_long
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() + Policy::genesis_block_number(),
                true,
            )
            .unwrap();
        let epoch2 = chain_long
            .read()
            .get_block_at(
                Policy::blocks_per_epoch() * 2 + Policy::genesis_block_number(),
                true,
            )
            .unwrap();

        // Short server: the same chain truncated to two epochs (head = epoch-2 election block).
        let chain_short = blockchain();
        if let BlockchainProxy::Full(ref chain_short) = chain_short {
            assert_eq!(
                Blockchain::push_macro(chain_short.upgradable_read(), epoch1.clone()),
                Ok(PushResult::Extended),
            );
            assert_eq!(
                Blockchain::push_macro(chain_short.upgradable_read(), epoch2.clone()),
                Ok(PushResult::Extended),
            );
        }

        // Client: at epoch-2 with `previous_slots` cleared and the epoch-1 block removed, so the
        // backward fetch is genuinely required (its predecessor is not on-chain).
        if let BlockchainProxy::Full(ref chain1) = chain1 {
            assert_eq!(
                Blockchain::push_macro(chain1.upgradable_read(), epoch1.clone()),
                Ok(PushResult::Extended),
            );
            assert_eq!(
                Blockchain::push_macro(chain1.upgradable_read(), epoch2.clone()),
                Ok(PushResult::Extended),
            );
            let mut chain1_wg = chain1.write();
            let mut txn = chain1_wg.write_transaction();
            chain1_wg.chain_store.remove_chain_info(
                &mut txn,
                &epoch1.hash(),
                epoch1.block_number(),
            );
            txn.commit();
            chain1_wg.state.previous_slots = None;
        }

        let mut sync = LightMacroSync::<MockNetwork>::new(
            chain1.clone(),
            Arc::clone(&net1),
            net1.subscribe_events(),
            0,
            None,
        );

        spawn_request_handlers(&net_short, &chain_short.clone());
        spawn_request_handlers(&net_long, &chain_long.clone());
        net1.dial_mock(&net_short);
        net1.dial_mock(&net_long);

        // The short peer (synced with our epoch-2 head) may legitimately be emitted `Good` once
        // `previous_slots` is set, before the forward sync to epoch-3 completes; keep driving until
        // our head reaches the long server's.
        loop {
            match sync.next().await {
                Some(MacroSyncReturn::Good(_)) | Some(MacroSyncReturn::Outdated(_)) => {
                    if chain1.read().head_hash() == chain_long.read().head_hash() {
                        break;
                    }
                }
                res => panic!("Unexpected MacroSyncReturn: {res:?}"),
            }
        }
        assert_eq!(
            chain1.read().previous_validators(),
            chain_long.read().previous_validators()
        );
    }

    /// A macro-block push that fails because the block doesn't fit our chain (missing predecessor
    /// or fork) must be classified as non-malicious, while an intrinsically invalid block must not.
    /// This is what keeps the queue from banning a peer that merely relayed a hash-matching block
    /// whose predecessor we failed to fetch.
    #[test]
    fn unfitting_push_errors_are_not_treated_as_malicious() {
        use nimiq_block::BlockError;
        use nimiq_blockchain_interface::PushError;

        use super::is_unfitting_push_error;

        // Block doesn't extend our chain right now — the serving peer is not at fault.
        for error in [
            PushError::Orphan,
            PushError::InvalidSuccessor,
            PushError::InvalidPredecessor,
            PushError::InvalidFork,
            PushError::InvalidBlock(BlockError::InvalidParentElectionHash),
            PushError::InvalidBlock(BlockError::InvalidBlockNumber),
        ] {
            assert!(
                is_unfitting_push_error(&error),
                "{error:?} should be treated as unfitting (no ban)"
            );
        }

        // Intrinsically invalid block — the peer must be banned.
        for error in [
            PushError::InvalidBlock(BlockError::InvalidJustification),
            PushError::InvalidBlock(BlockError::BodyHashMismatch),
            PushError::InvalidBlock(BlockError::InvalidSeed),
            PushError::DuplicateTransaction,
        ] {
            assert!(
                !is_unfitting_push_error(&error),
                "{error:?} should be treated as a ban"
            );
        }
    }
}
