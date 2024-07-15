use std::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    future::{BoxFuture, Future, FutureExt},
    stream::{BoxStream, Stream, StreamExt},
};
use nimiq_time::{interval, Interval};

use crate::{
    config::Config,
    contribution::AggregatableContribution,
    identity::IdentityRegistry,
    level::Level,
    network::{LevelUpdateSender, Network},
    partitioner::Partitioner,
    protocol::Protocol,
    store::ContributionStore,
    todo::{TodoItem, TodoList},
    update::LevelUpdate,
    verifier::{VerificationResult, Verifier},
    Identifier,
};

// TODOS:
// * protocol.store RwLock.
// * level.state RwLock
// * Evaluator::new -> threshold (now covered outside of this crate)

type LevelUpdateStream<P, T> = BoxStream<'static, LevelUpdate<<P as Protocol<T>>::Contribution>>;

/// Future implementation for the next aggregation event
pub struct OngoingAggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    /// Handel configuration
    config: Config,

    /// Levels
    levels: Vec<Level>,

    /// Remaining Todos
    todos: TodoList<TId, P>,

    /// The protocol specifying how this aggregation works.
    protocol: P,

    /// Our contribution
    contribution: P::Contribution,

    /// Sink used to relay messages
    sender: LevelUpdateSender<N>,

    /// Interval for starting the next level regardless of previous levels completion
    start_level_interval: Interval,

    /// Interval for sending level updates to the corresponding peers regardless of progression
    periodic_update_interval: Interval,

    /// the level which needs activation next
    next_level_timeout: usize,

    /// Future of the currently verified todo. There is only ever one todo being verified at a time.
    current_verification:
        Option<BoxFuture<'static, (VerificationResult, TodoItem<P::Contribution>)>>,
}

impl<TId, P, N> OngoingAggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    pub fn new(
        protocol: P,
        config: Config,
        own_contribution: P::Contribution,
        input_stream: LevelUpdateStream<P, TId>,
        sender: LevelUpdateSender<N>,
    ) -> Self {
        // Invoke the partitioner to create the level structure of peers.
        let levels: Vec<Level> = Level::create_levels(protocol.partitioner(), protocol.identify());

        // Create an empty todo list which can later be polled for the best available todo.
        let mut todos = TodoList::new(protocol.identify(), protocol.evaluator(), input_stream);

        // Add our own contribution to the todo list.
        todos.add_contribution(own_contribution.clone(), 0);

        // Regardless of level completion consecutive levels need to be activated at some point. Activate Levels every time this interval ticks,
        // if the level has not already been activated due to level completion
        let start_level_interval = interval(config.timeout);

        // Every `config.update_interval` send Level updates to corresponding peers no matter the aggregations progression
        // (makes sure other peers can catch up).
        let periodic_update_interval = interval(config.update_interval);

        // Create the NextAggregation struct
        Self {
            protocol,
            config,
            todos,
            levels,
            contribution: own_contribution,
            sender,
            start_level_interval,
            periodic_update_interval,
            next_level_timeout: 0,
            current_verification: None,
        }
    }

    /// Starts level `level`
    fn start_level(&mut self, level: usize, store: &<P as Protocol<TId>>::Store) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Attempted to start invalid level {level}"));
        trace!(
            id = ?self.protocol.identify(),
            ?level,
            "Starting level",
        );

        // Try to Start the level
        if level.start() {
            // In case the level was not started previously send the best contribution to peers on the level

            // Don't do anything for level 0 as it only contains this node
            if level.id > 0 {
                // Get the current best for the level. Freeing the lock as soon as possible to continue working on todos
                let best = store.combined(level.id - 1);

                if let Some(best) = best {
                    self.send_update(
                        best,
                        level.id,
                        !level.receive_complete(),
                        level.select_next_peers(self.config.peer_count),
                    );
                }
            }
        }
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

    /// Check if a level was completed
    fn check_completed_level(&mut self, level_id: usize, store: &<P as Protocol<TId>>::Store) {
        let num_peers = {
            let level = self
                .levels
                .get(level_id)
                .expect("Attempted to check completeness of invalid level");

            // check if level already is completed
            if level.state.read().receive_completed {
                // The level was completed before so nothing more to do.
                return;
            }

            level.num_peers()
        };

        if num_peers == 0 {
            trace!("Level {} is empty and thus complete", level_id);
            return;
        }

        // first get the current contributor count for this level. Release the lock as soon as possible
        // to continue working on todos.
        let best = store
            .best(level_id)
            .unwrap_or_else(|| panic!("Expected a best signature for level {}", level_id));
        let num_contributors = self.num_contributors(best);

        // If the number of contributors on this level is equal to the number of peers on this level it is completed.
        if num_contributors == num_peers {
            trace!(
                id = ?self.protocol.identify(),
                level_id,
                "Level complete",
            );
            {
                // Acquire write lock and set the level state for this level to completed.
                self.levels
                    .get(level_id)
                    .unwrap() // would have panicked earlier.
                    .state
                    .write()
                    .receive_completed = true;
            }
            // if there is a level with a higher id than the completed one it needs to be activated.
            if level_id + 1 < self.levels.len() {
                // activate next level
                self.start_level(level_id + 1, store);
            }
        }

        // In order to send updated messages iterate all levels higher than the given level.
        let level_count = self.levels.len();
        for i in level_id + 1..level_count {
            let combined = store.combined(i - 1);

            // if there is an aggregate contribution for given level i send it out to the peers of that level.
            if let Some(multisig) = combined {
                let level = self.levels.get(i).unwrap_or_else(|| panic!("No level {i}"));
                if level.update_signature_to_send(&multisig.clone()) {
                    self.send_update(
                        multisig,
                        level.id,
                        !level.receive_complete(),
                        level.select_next_peers(self.config.peer_count),
                    );
                }
            }
        }
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
        peer_ids: Vec<usize>,
    ) {
        // If there are peers to send the update to send them
        if !peer_ids.is_empty() {
            // If the send_individual flag is set the individual contribution is sent alongside the aggregate.
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

            // Send the level update to every peer_id in peer_ids
            for peer_id in peer_ids {
                // This should always be the case
                if peer_id < self.protocol.partitioner().size() {
                    self.sender
                        // `send` is not a future and thus will not block execution.
                        .send((update.clone(), peer_id));
                }
            }
        }
    }

    /// Send updates for every level to every peer accordingly.
    fn automatic_update(&mut self) {
        // Skip level 0 since it only contains this node
        for level_id in 1..self.levels.len() {
            let (receive_complete, next_peers) = {
                let level = self.levels.get(level_id).unwrap();
                (
                    level.receive_complete(),
                    level.select_next_peers(self.config.peer_count),
                )
            };

            // Get the current best aggregate from store (no clone() needed as that already happens within the store)
            // freeing the lock as soon as possible for the todo aggregating to continue.
            let aggregate = {
                let store = self.protocol.store();
                let store = store.read();
                store.combined(level_id - 1)
            };

            // For an existing aggregate for this level send it around to the respective peers.
            if let Some(aggregate) = aggregate {
                self.send_update(aggregate, level_id, !receive_complete, next_peers);
            }
        }
    }

    /// activate the next level which needs to be activated.
    fn activate_next_level(&mut self) {
        // the next level which needs activating on timeout.
        let level = self.next_level_timeout;
        // make sure such level exists
        if level < self.levels.len() {
            trace!(
                id = ?self.protocol.identify(),
                ?level,
                "Timeout at level",
            );

            // next time the timeout triggers the next level needs activating
            self.next_level_timeout += 1;

            let store_rw = self.protocol.store();
            let store = store_rw.read();

            // finally start the level.
            self.start_level(level, &store);
        }
    }

    fn into_inner(self) -> (LevelUpdateStream<P, TId>, LevelUpdateSender<N>) {
        (self.todos.into_stream(), self.sender)
    }

    /// Applies the given todo. It will either be immediately emitted as the new best aggregate if it is completed
    /// or it will be added to the store and the new best aggregate will be emitted.
    fn apply_todo(&mut self, todo: TodoItem<P::Contribution>) -> P::Contribution {
        // special case of full contributions
        if todo.level == self.protocol.partitioner().levels()
            && self.is_complete_aggregate(&todo.contribution)
        {
            return todo.contribution;
        }

        let store_rw = self.protocol.store();
        let mut store = store_rw.write();

        // if the contribution is valid push it to the store, creating a new aggregate
        store.put(
            todo.contribution.clone(),
            todo.level,
            self.protocol.registry(),
            self.protocol.identify(),
        );

        // in case the level of this todo has not started, start it now as we have already contributions on it.
        self.start_level(todo.level, &store);
        // check if a level was completed by the addition of the contribution
        self.check_completed_level(todo.level, &store);

        // get the best aggregate
        let last_level = self.levels.last().expect("No levels");

        let best = store.combined(last_level.id);

        best.expect("A best signature must exist after applying a contribution.")
    }

    /// Verifies a given signature.
    ///
    /// As signature verification can be costly it creates a future which will be polled directly after creation.
    /// The created future may return immediately.
    ///
    /// This function will override `self.current_verification` if the created future does not immediately produce a value.
    ///
    /// ## Returns
    /// Returns the verification result and the todo in question iff the verification resolves immediately.
    /// None otherwise
    fn start_todo_verification(
        &mut self,
        todo: TodoItem<P::Contribution>,
        cx: &mut Context<'_>,
    ) -> Option<(VerificationResult, TodoItem<P::Contribution>)> {
        // Create a new verification future.
        let verifier = self.protocol.verifier();
        let mut fut = async move {
            let result = verifier.verify(&todo.contribution).await;
            (result, todo)
        }
        .boxed();

        if let Poll::Ready(result) = fut.poll_unpin(cx) {
            return Some(result);
        }

        self.current_verification = Some(fut);
        None
    }
}

impl<TId, P, N> Stream for OngoingAggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    type Item = P::Contribution;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Poll the verification future if there is one.
        let mut best_aggregate = if let Some(verification_future) = &mut self.current_verification {
            if let Poll::Ready((result, todo)) = verification_future.poll_unpin(cx) {
                // If a result is produced, unset the future such that a new one can take its place.
                self.current_verification = None;
                if result.is_ok() {
                    // If the todo was successfully verified, apply it and return the new best aggregate
                    Some(self.apply_todo(todo))
                } else {
                    // If the Todo failed to verify there is nothing to be done, and also no new best aggregate.
                    log::debug!(?result, ?todo, "Verification of Todo Item failed.");
                    None
                }
            } else {
                // The future has not yet resolved. There is no new best aggregate.
                None
            }
        } else {
            // There is no future to poll and thus no new todo to process. There is no new best aggregate.
            None
        };

        // Check if the automatic update interval triggers, if so perform the update.
        while let Poll::Ready(_instant) = self.periodic_update_interval.as_mut().poll_tick(cx) {
            // This creates new messages in the sender.
            self.automatic_update();
        }

        while let Poll::Ready(_instant) = self.start_level_interval.as_mut().poll_tick(cx) {
            // Activates the next level if there is a next level.
            // This potentially creates new messages in the sender.
            self.activate_next_level();
        }

        // Check and see if a new todo should be verified.
        // Only start a new verification task if there is not already one and also only if the poll does not have produced a value yet.
        // This is necessary as the verification future could resolve immediately producing a second item for the stream.
        // As the new best aggregate will be returned this stream will be polled again creating the future in the next poll
        if self.current_verification.is_none() && best_aggregate.is_none() {
            // Get the next best todo.
            while let Poll::Ready(Some(todo)) = self.todos.poll_next_unpin(cx) {
                // Start the verification. This will also poll and thus may immediately return a result.
                if let Some((result, todo)) = self.start_todo_verification(todo, cx) {
                    // If the future returned immediately it needs to be handled.
                    if result.is_ok() {
                        // If the todo verified, apply it and return the new best aggregate
                        let new_aggregate = self.apply_todo(todo);

                        best_aggregate = Some(new_aggregate);
                        break;
                    } else {
                        log::debug!(?result, ?todo, "Verification of Todo Item failed.");
                    }
                } else {
                    break;
                }
            }
        }

        // Poll the sender, sending as many messages as possible.
        assert!(self.sender.poll_unpin(cx).is_pending());

        // Return the aggregate if available.
        if let Some(contribution) = best_aggregate {
            return Poll::Ready(Some(contribution));
        }

        // Return Pending otherwise.
        Poll::Pending
    }
}

pub struct FinishedAggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    level_update: LevelUpdate<P::Contribution>,
    input_stream: LevelUpdateStream<P, TId>,
    sender: LevelUpdateSender<N>,
}

impl<TId, P, N> FinishedAggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    fn from(aggregation: OngoingAggregation<TId, P, N>, aggregate: P::Contribution) -> Self {
        // create a level update from the final aggregation
        let level_update = LevelUpdate::<P::Contribution>::new(
            aggregate,
            None,
            aggregation.protocol.partitioner().levels(),
            aggregation.protocol.node_id(),
        );

        // Get rid of the aggregation retaining the input stream of level updates and the sender.
        let (input_stream, sender) = aggregation.into_inner();

        Self {
            level_update,
            input_stream,
            sender,
        }
    }
}

impl<TId, P, N> Future for FinishedAggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    /// The type could just be `()` as it will only ever return `Poll::Pending`, but for compatibility with the
    /// `OngoingAggregation` it is kept. As it is a Future the Option is added.
    type Output = Option<P::Contribution>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Poll::Ready(msg) = self.input_stream.poll_next_unpin(cx) {
            if let Some(msg) = msg {
                // Don't respond to final level updates.
                if msg.level == self.level_update.level {
                    continue;
                }

                let response = (self.level_update.clone(), msg.origin());
                self.sender.send(response);
            } else {
                log::error!("Failed to poll input stream; None returned")
            }
        }

        // Send as many messages as possible.
        assert!(self.sender.poll_unpin(cx).is_pending());

        // Do never produce an item as it has been produced earlier already.
        // This future will never produce an item.
        Poll::Pending
    }
}

/// Abstraction for both kinds of aggregations.
/// There is an additional variant used to transition from ongoing to finished.
pub enum Aggregation<TId, P, N>
where
    TId: Identifier,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
{
    /// The aggregation is currently ongoing and it is going to produce stream items as well as network traffic.
    Ongoing(OngoingAggregation<TId, P, N>),
    /// The aggregation is finished and it will no longer produce stream items and it will reduce network traffic
    /// to answering messages with an unfinished aggregate within them.
    Finished(FinishedAggregation<TId, P, N>),
    /// The aggregation has just finished and is transitioning from ongoing to finished.
    /// Used to deconstruct the OngoingAggregation and should be `unreachable!()` everywhere but in that location.
    Transitioning,
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
        input_stream: LevelUpdateStream<P, TId>,
        network: N,
    ) -> Self {
        // Create the Sender, buffering a single message per recipient.
        let sender = LevelUpdateSender::new(protocol.partitioner().size(), network);

        // Aggregations start out as Ongoing
        Self::Ongoing(OngoingAggregation::new(
            protocol,
            config,
            own_contribution,
            input_stream,
            sender,
        ))
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
        // Poll either the ongoing or the finished aggregation
        let result = match &mut *self {
            Self::Transitioning => {
                unreachable!("Aggregation should never be transitioning when polled.")
            }
            Self::Finished(finished_aggregation) => {
                // Finished aggregations are simply polled as the future will never produce a value.
                assert!(finished_aggregation.poll_unpin(cx).is_pending());
                return Poll::Pending;
            }
            Self::Ongoing(ongoing_aggregation) => {
                let result = ongoing_aggregation.poll_next_unpin(cx);
                // For produced stream items of the ongoing aggregations it needs to be checked if the transition into a
                // finished aggregation is required.
                if let Poll::Ready(Some(aggregate)) = &result {
                    // Only completely full aggregates trigger the transition.
                    if ongoing_aggregation.is_complete_aggregate(aggregate) {
                        // Take the ongoing aggregation out of *self, replacing it with the Transitioning variant.
                        let ongoing_aggregation = mem::replace(&mut *self, Self::Transitioning);
                        let ongoing_aggregation = match ongoing_aggregation {
                            Self::Ongoing(aggregation) => aggregation,
                            _ => panic!("Was ongoing before, should still be ongoing."),
                        };
                        // Create the FinishedAggregation and replace the Transitioning variant with it again.
                        *self = Self::Finished(FinishedAggregation::from(
                            ongoing_aggregation,
                            aggregate.clone(),
                        ));
                    }
                }
                result
            }
        };

        // In case there is now a FinishedAggregation where before there wasn't poll it, to register the waker.
        if let Self::Finished(finished_aggregation) = &mut *self {
            assert!(finished_aggregation.poll_unpin(cx).is_pending());
        }

        result
    }
}
