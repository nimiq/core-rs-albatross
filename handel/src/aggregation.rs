use std::{
    pin::Pin,
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
    identity::{Identity, IdentityRegistry},
    level::Level,
    network::{Network, NetworkHelper},
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
    levels: Vec<Level>,

    /// Stream of level updates received from other peers.
    input_stream: LevelUpdateStream<P::Contribution>,

    /// List of remaining pending contributions
    pending_contributions: PendingContributionList<TId, P>,

    /// The protocol specifying how this aggregation works.
    protocol: P,

    /// Our contribution
    contribution: P::Contribution,

    /// Gateway to the network to send updates and ban nodes.
    network: NetworkHelper<N>,

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
        let sender = NetworkHelper::new(protocol.partitioner().size(), network);

        // Invoke the partitioner to create the level structure of peers.
        let levels = Level::create_levels(
            protocol.partitioner(),
            protocol.identify(),
            protocol.node_id(),
        );

        // Create an empty list which can later be polled for the best available pending contribution.
        let mut pending_contributions =
            PendingContributionList::new(protocol.identify(), protocol.evaluator());

        // Add our own contribution to the list.
        pending_contributions.add_contribution(own_contribution.clone(), 0, protocol.node_id());

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
            network: sender,
            start_level_interval,
            periodic_update_interval,
            current_verification: None,
            final_result: None,
        }
    }

    /// Starts all levels up to `level`.
    fn start_level(&mut self, level: usize, store: &P::Store, activated_by: &'static str) {
        // Find the first level that has not been started yet.
        // If all levels are already started, there is nothing to do.
        let Some(to_start) = self.levels.iter().position(|level| !level.is_started()) else {
            return;
        };

        // If we have already started a higher level, there is nothing to do.
        if to_start > level {
            return;
        }

        // Activate all levels between `to_start` (inclusive) and `level` (inclusive).
        for lvl in to_start..=level {
            // Start the level.
            let level = &self.levels[lvl];
            level.start();

            debug!(
                id = %self.protocol.identify(),
                level = level.id,
                activated_by,
                "Starting level"
            );

            // Nothing else to do for level 0 as it only contains this node.
            if level.id == 0 {
                continue;
            }

            // Send the best aggregate to the peers on the level.
            self.send_update(
                level.id,
                level.select_next_peers(self.config.peer_count),
                store,
            );
        }

        // Reset the level timeout.
        self.start_level_interval = interval(self.config.level_timeout);
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
    fn check_completed_level(&mut self, level_id: usize, store: &P::Store) {
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
            id = %self.protocol.identify(),
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
        } else {
            self.send_update(
                next_level_id,
                next_level.select_next_peers(self.config.peer_count),
                store,
            );
        }

        // Recursively check if the next level was completed as well.
        self.check_completed_level(next_level_id, store);
    }

    /// Send updated `contribution` for `level` to `count` peers
    ///
    /// If the `send_individual` flag is set, the contribution containing solely
    /// this node contribution is sent alongside the aggregate.
    fn send_update(&mut self, mut level_id: usize, node_ids: Identity, store: &P::Store) {
        // Nothing to do if there are no recipients.
        if node_ids.is_empty() {
            return;
        }

        // If we already have a final result for this aggregation, return it.
        // Otherwise, return our current best aggregate for the peer's level.
        let contribution = if let Some(final_result) = &self.final_result {
            level_id = self.levels.len();
            final_result.clone()
        } else {
            let Some(best_aggregate) = store.combined(level_id - 1) else {
                return;
            };
            best_aggregate
        };

        // FIXME Comment
        let expected_contributors = self
            .protocol
            .partitioner()
            .cumulative_level_size(level_id - 1);

        let num_contributors = self.num_contributors(&contribution);

        let individual = if num_contributors < expected_contributors {
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
        for node_id in node_ids.iter() {
            debug!(
                from = self.protocol.node_id(),
                to = ?node_id,
                level = level_id,
                signers = %update.aggregate.contributors(),
                "Sending level update",
            );
            self.network.send(node_id, update.clone());
        }
    }

    /// Send updates for every level to every peer accordingly.
    fn automatic_update(&mut self) {
        let store = self.protocol.store();
        let store = store.read();

        // Skip level 0 since it only contains this node.
        for level_id in 1..self.levels.len() {
            // Skip levels that aren't started yet.
            let level = &self.levels[level_id];
            if !level.is_started() {
                break;
            }

            // Send the aggregate to the next peers on this level.
            let next_peers = level.select_next_peers(self.config.peer_count);
            self.send_update(level_id, next_peers, &store);
        }
    }

    /// Activate the next level that has not started yet.
    fn activate_next_level(&mut self) {
        // Find the first level that has not been started yet.
        let to_start = self.levels.iter().position(|level| !level.is_started());

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

        // Put the contribution into the store, creating a new aggregate.
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

    /// Responds to an update received from `node_id` with our own best aggregate for `level_id`.
    /// If we already have a final result, we send that result instead.
    fn respond_to_update(&mut self, node_id: usize, level_id: usize) {
        // If the peer already has a full aggregation, we don't need to respond.
        if level_id == self.levels.len() {
            return;
        }

        // Respond with our current best aggregate.
        let store = self.protocol.store();
        let store = store.read();
        self.send_update(level_id, Identity::single(node_id), &store);
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
            // Verify the level update.
            if let Err(error) = evaluator.verify(&update) {
                warn!(
                    id = %self.protocol.identify(),
                    ?error,
                    ?update,
                    "Rejecting invalid level update",
                );
                // TODO Ban peer?
                continue;
            }

            // Respond with our own update.
            let origin = update.origin();
            let level = update.level();
            self.respond_to_update(origin, level);

            // Store the aggregate (and individual if present) contributions in `pending_contributions`.
            self.pending_contributions
                .add_contribution(update.aggregate, level, origin);
            if let Some(individual) = update.individual {
                self.pending_contributions
                    .add_contribution(individual, level, origin);
            }
        }

        // Poll the verification future if there is one.
        if let Some(future) = &mut self.current_verification {
            if let Poll::Ready((result, contribution)) = future.poll_unpin(cx) {
                // If a result is produced, unset the future such that a new one can take its place.
                self.current_verification = None;

                if result.is_ok() {
                    // If the contribution was successfully verified, apply it and return the new
                    // best aggregate.
                    best_aggregate = Some(self.apply_contribution(contribution))
                } else {
                    // Verification failed, ban sender.
                    warn!(
                        id = %self.protocol.identify(),
                        ?result,
                        ?contribution,
                        "Rejecting invalid contribution"
                    );
                    self.network.ban_node(contribution.origin);
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
                    // Verification failed, ban sender.
                    warn!(
                        id = %self.protocol.identify(),
                        ?result,
                        ?contribution,
                        "Rejecting invalid contribution"
                    );
                    self.network.ban_node(contribution.origin);
                }
            }
        }

        // Drive the network helper future. It always returns Pending.
        let _ = self.network.poll_unpin(cx);

        // If this aggregation has already produced a final result, we return Pending instead of
        // Ready(None) to allow this aggregation to keep running so that it can send full aggregations
        // to other nodes. This also means that the aggregation stream will never terminate.
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
                id = %self.protocol.identify(),
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
        Poll::Ready(Some(best_aggregate))
    }
}
