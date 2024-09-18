use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    future::{BoxFuture, FutureExt},
    stream::{BoxStream, Stream, StreamExt},
};
use nimiq_time::{interval, Interval};

use crate::{
    config::Config,
    contribution::AggregatableContribution,
    evaluator::Evaluator,
    identity::IdentityRegistry,
    level::Level,
    network::{LevelUpdateSender, Network},
    partitioner::Partitioner,
    pending_contributions::{PendingContribution, PendingContributionList},
    protocol::Protocol,
    store::ContributionStore,
    update::LevelUpdate,
    verifier::{VerificationResult, Verifier},
    Identifier,
};

// TODOS:
// * protocol.store RwLock.
// * level.state RwLock
// * Evaluator::new -> threshold (now covered outside of this crate)

type LevelUpdateStream<C> = BoxStream<'static, LevelUpdate<C>>;

/// Future implementation for the next aggregation event
pub struct Aggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    /// Handel configuration
    config: Config,

    /// Levels
    levels: Arc<Vec<Level>>,

    /// Stream of level updates received from other peers.
    input_stream: LevelUpdateStream<P::Contribution>,

    /// List of remaining pending contributions
    pending_contributions: PendingContributionList<TId, P>,

    /// The protocol specifying how this aggregation works.
    protocol: P,

    /// Our contribution
    contribution: P::Contribution,

    /// Sink used to relay messages
    sender: LevelUpdateSender<N>,

    /// Timeout for starting the next level regardless of previous level's completion.
    start_level_interval: Interval,

    /// Interval for sending level updates to the corresponding peers regardless of progression.
    periodic_update_interval: Interval,

    /// Future of the currently verified pending contribution.
    /// There is only ever one contribution being verified at a time.
    current_verification:
        Option<BoxFuture<'static, (VerificationResult, PendingContribution<P::Contribution>)>>,

    /// The final result of the aggregation once it has been produced.
    /// A Some(_) value here indicates that the aggregation has finished.
    final_result: Option<P::Contribution>,
}

impl<TId, P, N> Aggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    pub fn new(
        protocol: P,
        config: Config,
        own_contribution: P::Contribution,
        input_stream: LevelUpdateStream<P::Contribution>,
        network: N,
    ) -> Self {
        // Create the Sender, buffering a single message per recipient.
        let sender = LevelUpdateSender::new(protocol.partitioner().size(), network);

        // Invoke the partitioner to create the level structure of peers.
        let levels = Arc::new(Level::create_levels(
            protocol.partitioner(),
            protocol.identify(),
            protocol.node_id(),
        ));

        // Create an empty list which can later be polled for the best available pending contribution.
        let mut pending_contributions =
            PendingContributionList::new(protocol.identify(), protocol.evaluator());

        // Add our own contribution to the list.
        pending_contributions.add_contribution(own_contribution.clone(), 0);

        // Regardless of level completion consecutive levels need to be activated at some point.
        // Activate levels every time this interval ticks, if the level has not already been
        // activated due to level completion.
        let start_level_interval = interval(config.level_timeout);

        // Every `config.update_interval` send level updates to corresponding peers no matter the
        // aggregation's progression (makes sure other peers can catch up).
        let periodic_update_interval = interval(config.update_interval);

        Self {
            protocol,
            config,
            pending_contributions,
            levels,
            input_stream,
            contribution: own_contribution,
            sender,
            start_level_interval,
            periodic_update_interval,
            current_verification: None,
            final_result: None,
        }
    }

    /// Starts level `level`
    fn start_level(
        &mut self,
        level: usize,
        store: &<P as Protocol<TId>>::Store,
        activated_by: &'static str,
    ) {
        // Try to start the level. `level.start()` returns false if the level was already started.
        let level = &self.levels[level];
        if !level.start() {
            return;
        }

        debug!(
            id = ?self.protocol.identify(),
            level = level.id,
            activated_by,
            "Starting level"
        );

        // Reset the level timeout.
        self.start_level_interval = interval(self.config.level_timeout);

        // Nothing else to do for level 0 as it only contains this node.
        if level.id == 0 {
            return;
        }

        // Get the current best aggregate for the level.
        let Some(best) = store.combined(level.id - 1) else {
            return;
        };

        // Send the best aggregate to the peers on the level.
        self.send_update(
            best,
            level.id,
            !level.is_complete(),
            level.select_next_peers(self.config.peer_count),
        );
    }

    fn num_contributors(&self, aggregate: &P::Contribution) -> usize {
        self.protocol
            .registry()
            .signers_identity(&aggregate.contributors())
            .len()
    }

    fn is_complete_aggregate(&self, aggregate: &P::Contribution) -> bool {
        self.num_contributors(aggregate) == self.protocol.partitioner().size()
    }

    /// Check if the given level was completed.
    fn check_completed_level(&mut self, level_id: usize, store: &<P as Protocol<TId>>::Store) {
        // Nothing to do if the level is already complete.
        let level = &self.levels[level_id];
        if level.is_complete() {
            return;
        }

        // Check if the level is complete.
        // Levels with 0 peers are always considered complete.
        let num_peers = level.num_peers();
        if num_peers > 0 {
            // Get the number of contributors that we have stored for this level.
            let Some(best) = store.best(level_id) else {
                return;
            };
            let num_contributors = self.num_contributors(best);

            // If we don't have enough contributors, the level is not complete yet.
            if num_contributors < num_peers {
                return;
            }
        }

        trace!(
            id = ?self.protocol.identify(),
            level_id,
            num_peers,
            "Level complete",
        );

        // Mark the level as complete.
        self.levels[level_id].state.write().complete = true;

        // If this level is the last one, we're done.
        if level_id == self.levels.len() - 1 {
            return;
        }

        // If the next level has not been started yet, start it (which also immediately sends an
        // update), otherwise send the improved contribution.
        let next_level_id = level_id + 1;
        let next_level = &self.levels[next_level_id];

        if !next_level.is_started() {
            self.start_level(next_level_id, store, "LevelComplete");
        } else if let Some(best_aggregate) = store.combined(next_level_id - 1) {
            self.send_update(
                best_aggregate,
                next_level_id,
                true,
                next_level.select_next_peers(self.config.peer_count),
            );
        }

        // Recursively check if the next level was completed as well.
        self.check_completed_level(next_level_id, store);
    }

    /// Send updated `contribution` for `level` to `count` peers
    ///
    /// If the `send_individual` flag is set, the contribution containing solely
    /// this node contribution is sent alongside the aggregate.
    fn send_update(
        &mut self,
        contribution: P::Contribution,
        level_id: usize,
        send_individual: bool,
        node_ids: Vec<usize>,
    ) {
        // Nothing to do if there are no recipients.
        if node_ids.is_empty() {
            return;
        }

        // If the send_individual flag is set, the individual contribution is sent alongside the aggregate.
        let individual = if send_individual {
            Some(self.contribution.clone())
        } else {
            None
        };

        // Create the LevelUpdate
        let update = LevelUpdate::<P::Contribution>::new(
            contribution,
            individual,
            level_id,
            self.protocol.node_id(),
        );

        // Send the level update to every node_id in node_ids.
        for node_id in node_ids {
            self.sender.send(node_id, update.clone());
            debug!(
                from = self.protocol.node_id(),
                to = node_id,
                level = level_id,
                signers = %update.aggregate.contributors(),
                "Sending level update",
            );
        }
    }

    /// Send updates for every level to every peer accordingly.
    fn automatic_update(&mut self) {
        // Skip level 0 since it only contains this node.
        for level_id in 1..self.levels.len() {
            // Skip levels that are complete or that haven't started yet.
            let level = &self.levels[level_id];
            if level.is_complete() || !level.is_started() {
                continue;
            }

            // Get the current best aggregate for this level from the store.
            let aggregate = {
                let store = self.protocol.store();
                let store = store.read();
                let Some(aggregate) = store.combined(level_id - 1) else {
                    continue;
                };
                aggregate
            };

            // Send the aggregate to the next peers on this level.
            let next_peers = level.select_next_peers(self.config.peer_count);
            self.send_update(aggregate, level_id, true, next_peers);
        }
    }

    /// Activate the next level that has not started yet.
    fn activate_next_level(&mut self) {
        // Find the best completed level.
        let best_complete = self
            .levels
            .iter()
            .rposition(|level| level.is_complete())
            .unwrap_or(0);

        // If the best completed level is the last level, we're done.
        if best_complete == self.levels.len() - 1 {
            return;
        }

        // Now find the first level above the best completed level that has not been started yet.
        let to_start = self.levels[best_complete + 1..]
            .iter()
            .position(|level| !level.is_started());

        // If there are no levels to start, we're done.
        let Some(to_start) = to_start else {
            return;
        };

        // Start the level.
        let store = self.protocol.store();
        self.start_level(to_start, &store.read(), "Timeout");
    }

    /// Applies the given pending contribution. It will either be immediately emitted as the new
    /// best aggregate if it is completed, or it will be added to the store and the new best
    /// aggregate will be emitted.
    fn apply_contribution(
        &mut self,
        contribution: PendingContribution<P::Contribution>,
    ) -> P::Contribution {
        // Special case for full aggregations, which are sent at level `num_levels`.
        if contribution.level == self.levels.len() {
            return contribution.contribution;
        }

        let store = self.protocol.store();
        let mut store = store.write();

        // if the contribution is valid push it to the store, creating a new aggregate
        store.put(
            contribution.contribution.clone(),
            contribution.level,
            self.protocol.registry(),
            self.protocol.identify(),
        );

        // If the level of this pending contribution has not started, start it now as we already
        // have contributions for it.
        self.start_level(contribution.level, &store, "ApplyContribution");

        // Check if a level was completed by the addition of the contribution.
        self.check_completed_level(contribution.level, &store);

        // Return the best aggregate.
        store
            .combined(self.levels.len() - 1)
            .expect("A best signature must exist after applying a contribution")
    }

    /// Verifies a given contribution.
    ///
    /// As signature verification can be costly it, creates a future which will be polled directly
    /// after creation. The created future may return immediately.
    ///
    /// This function will override `self.current_verification` if the created future does not
    /// immediately produce a value.
    ///
    /// Returns the [VerificationResult] and the [PendingContribution] in question if the
    /// verification resolves immediately, None otherwise.
    fn start_verification(
        &mut self,
        pending_contribution: PendingContribution<P::Contribution>,
        cx: &mut Context<'_>,
    ) -> Option<(VerificationResult, PendingContribution<P::Contribution>)> {
        // Create a new verification future.
        let verifier = self.protocol.verifier();
        let mut fut = async move {
            let result = verifier.verify(&pending_contribution.contribution).await;
            (result, pending_contribution)
        }
        .boxed();

        if let Poll::Ready(result) = fut.poll_unpin(cx) {
            return Some(result);
        }

        self.current_verification = Some(fut);
        None
    }

    fn respond_to_update(&mut self, node_id: usize, level_id: usize) {
        // TODO Only respond if our aggregate is better than the peer's or complements it.

        // Don't access `self.levels[level_id]` directly as it could point to `num_levels`, which
        // is the case for full aggregations. It's fine to just return, as the peer has already
        // finished aggregating and doesn't need our response anymore.
        let Some(level) = self.levels.get(level_id) else {
            return;
        };

        // If we already have a final result for this aggregation, return it.
        // Otherwise, return our current best aggregate for the peer's level.
        if let Some(final_result) = &self.final_result {
            self.send_update(
                final_result.clone(),
                self.levels.len(),
                false,
                vec![node_id],
            );
        } else {
            let store = self.protocol.store();
            let store = store.read();
            let Some(best_aggregate) = store.combined(level_id - 1) else {
                return;
            };

            self.send_update(
                best_aggregate,
                level_id,
                !level.is_complete(),
                vec![node_id],
            );
        };
    }
}

impl<TId, P, N> Stream for Aggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    type Item = P::Contribution;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut best_aggregate = None;

        // Poll the input stream for new level updates.
        let evaluator = self.protocol.evaluator();
        while let Poll::Ready(Some(update)) = self.input_stream.poll_next_unpin(cx) {
            // Verify that the sender of this update is on the correct level.
            if !evaluator.verify(&update) {
                trace!(
                    id = ?self.protocol.identify(),
                    ?update,
                    "Rejecting invalid level update",
                );
                continue;
            }

            // Respond with our own update.
            self.respond_to_update(update.origin(), update.level());

            // Store the aggregate (and individual if present) contributions in `pending_contributions`.
            let level = update.level();
            self.pending_contributions
                .add_contribution(update.aggregate, level);
            if let Some(individual) = update.individual {
                self.pending_contributions
                    .add_contribution(individual, level);
            }
        }

        // Poll the verification future if there is one.
        if let Some(future) = &mut self.current_verification {
            if let Poll::Ready((result, pending_contribution)) = future.poll_unpin(cx) {
                // If a result is produced, unset the future such that a new one can take its place.
                self.current_verification = None;

                if result.is_ok() {
                    // If the contribution was successfully verified, apply it and return the new
                    // best aggregate.
                    best_aggregate = Some(self.apply_contribution(pending_contribution))
                } else {
                    // TODO Ban peer who sent the failed contribution?
                    //  Or penalize it when scoring contributions?
                    debug!(
                        ?result,
                        ?pending_contribution,
                        "Verification of PendingContribution failed"
                    );
                }
            }
        };

        // Check if the automatic update interval triggers, if so perform the update.
        while let Poll::Ready(_instant) = self.periodic_update_interval.poll_next_unpin(cx) {
            // This creates new messages in the sender.
            self.automatic_update();
        }

        while let Poll::Ready(_instant) = self.start_level_interval.poll_next_unpin(cx) {
            // Activates the next level if there is a next level.
            // This potentially creates new messages in the sender.
            self.activate_next_level();
        }

        // Check and see if a new pending contribution should be verified.
        // Only start a new verification task if there is not already one and also only if the poll
        // does not have produced a value yet. This is necessary as the verification future could
        // resolve immediately producing a second item for the stream. As the new best aggregate
        // will be returned, this stream will be polled again creating the future in the next poll.
        if self.current_verification.is_none() && best_aggregate.is_none() {
            // Get the next best pending contribution.
            while let Poll::Ready(Some(contribution)) =
                self.pending_contributions.poll_next_unpin(cx)
            {
                // Start the verification. This will also poll and thus may immediately return a result.
                let Some((result, contribution)) = self.start_verification(contribution, cx) else {
                    break;
                };

                // If the future returned immediately, it needs to be handled.
                if result.is_ok() {
                    // If the contribution verified, apply it and return the new best aggregate.
                    best_aggregate = Some(self.apply_contribution(contribution));
                    break;
                } else {
                    // TODO Ban peer who sent the failed contribution?
                    //  Or penalize it when scoring contributions?
                    debug!(
                        ?result,
                        ?contribution,
                        "Verification of PendingContribution failed."
                    );
                }
            }
        }

        // Drive the level update sender future. It always returns Pending.
        let _ = self.sender.poll_unpin(cx);

        // If this aggregation has already produced a final result, we return Pending instead of
        // None to allow this aggregation to keep running so that it can send full aggregations to
        // other nodes. This also means that the aggregation stream will never terminate.
        if self.final_result.is_some() {
            return Poll::Pending;
        }

        // If we don't have a best aggregate, wait for more contributions.
        let Some(best_aggregate) = best_aggregate else {
            return Poll::Pending;
        };

        // If the best aggregate is a full aggregation, this aggregation is finished.
        if self.is_complete_aggregate(&best_aggregate) {
            debug!(
                id = ?self.protocol.identify(),
                "Aggregation complete"
            );

            // Store the final result, so that we can give it to other nodes.
            self.final_result = Some(best_aggregate.clone());

            // Mark all levels as complete to stop sending updates.
            for level in self.levels.iter() {
                level.state.write().complete = true;
            }
        }

        // Return the best aggregate.
        return Poll::Ready(Some(best_aggregate));
    }
}
