use std::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    future::{BoxFuture, Future, FutureExt},
    ready, select,
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
    todo::TodoList,
    update::LevelUpdate,
};

// TODOS:
// * protocol.store RwLock.
// * level.state RwLock
// * Evaluator::new -> threshold (now covered outside of this crate)

type LevelUpdateStream<P, T> = BoxStream<'static, LevelUpdate<<P as Protocol<T>>::Contribution>>;

/// Future implementation for the next aggregation event
struct NextAggregation<
    TId: Debug + Clone + Unpin + 'static,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
> {
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
}

impl<
        TId: Debug + Clone + Unpin + 'static,
        P: Protocol<TId>,
        N: Network<Contribution = P::Contribution>,
    > NextAggregation<TId, P, N>
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
        }
    }

    /// Starts level `level`
    fn start_level(&mut self, level: usize) {
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
                let best = {
                    let store = self.protocol.store();
                    let store = store.read();
                    store.combined(level.id - 1)
                };

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
    fn check_completed_level(&mut self, level_id: usize) {
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
        let num_contributors = {
            let store = self.protocol.store();
            let store = store.read();
            let best = store
                .best(level_id)
                .unwrap_or_else(|| panic!("Expected a best signature for level {}", level_id));
            self.num_contributors(best)
        };

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
                self.start_level(level_id + 1);
            }
        }

        // In order to send updated messages iterate all levels higher than the given level.
        let level_count = self.levels.len();
        for i in level_id + 1..level_count {
            let combined = {
                // Acquire read lock to retrieve the current combined contribution for the next lower level
                let store = self.protocol.store();
                let store = store.read();
                store.combined(i - 1)
            };

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

            // finally start the level.
            self.start_level(level);
        }
    }

    async fn next(mut self) -> (P::Contribution, Option<Self>) {
        // As long as there is no new aggregate to return loop over the select of both intervals and the actual aggregation
        loop {
            // Note that this function only gets called once per new aggregation.
            // That means levels can only be activated either by completing the previous one, or before starting a new todo, which is not ideal.
            // Likewise the periodic update will only trigger between todos.
            select! {
                _ = self.periodic_update_interval.next().fuse() => self.automatic_update(),
                _ = self.start_level_interval.next().fuse() => self.activate_next_level(),
                _ = self.sender.next().fuse() => {},
                item = self.todos.next().fuse() => {
                    match item {
                        Some(todo) => {
                            // verify the contribution
                            let result = self.protocol.verify(&todo.contribution).await;

                            if result.is_ok() {
                                // special case of full contributions
                                if todo.level == self.protocol.partitioner().levels() {
                                    return (todo.contribution, Some(self));
                                }

                                // if the contribution is valid push it to the store, creating a new aggregate
                                {
                                    let store = self.protocol.store();
                                    let mut store = store.write();
                                    store.put(
                                        todo.contribution.clone(),
                                        todo.level,
                                        self.protocol.registry(),
                                        self.protocol.identify(),
                                    );
                                }

                                // in case the level of this todo has not started, start it now as we have already contributions on it.
                                self.start_level(todo.level);
                                // check if a level was completed by the addition of the contribution
                                self.check_completed_level(todo.level);

                                // get the best aggregate
                                let last_level = self.levels.last().expect("No levels");
                                let best = {
                                    let store = self.protocol.store();
                                    let store = store.read();
                                    store.combined(last_level.id)
                                };

                                if let Some(best) = best {
                                    return (best, Some(self));
                                }
                            } else {
                                // Invalid contributions create a warning, but do not terminate. -> Continue with the next best todo item.
                                warn!(
                                    id = ?self.protocol.identify(),
                                    ?result,
                                    "Invalid signature",
                                );
                            }
                        },
                        None => {
                            // This should really never happen as TodoList<Item> does not return None.
                            panic!("Todo Stream returned None");
                        }
                    }
                },
            }
        }
    }

    fn into_inner(self) -> (LevelUpdateStream<P, TId>, LevelUpdateSender<N>) {
        (self.todos.into_stream(), self.sender)
    }
}

struct FinishedAggregation<T: Debug + Clone + 'static, P: Protocol<T>, N: Network> {
    level_update: LevelUpdate<P::Contribution>,
    input_stream: LevelUpdateStream<P, T>,
    sender: LevelUpdateSender<N>,
}

impl<
        TId: Debug + Clone + Unpin + Send + 'static,
        P: Protocol<TId>,
        N: Network<Contribution = P::Contribution>,
    > FinishedAggregation<TId, P, N>
{
    fn from(aggregation: NextAggregation<TId, P, N>, aggregate: P::Contribution) -> Self {
        let level_update = LevelUpdate::<P::Contribution>::new(
            aggregate,
            None,
            aggregation.protocol.partitioner().levels(),
            aggregation.protocol.node_id(),
        );

        let (input_stream, sender) = aggregation.into_inner();

        Self {
            level_update,
            input_stream,
            sender,
        }
    }
}

impl<
        TId: Debug + Clone + Unpin + Send + 'static,
        P: Protocol<TId>,
        N: Network<Contribution = P::Contribution>,
    > Future for FinishedAggregation<TId, P, N>
{
    type Output = (P::Contribution, Option<NextAggregation<TId, P, N>>);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // always returns Poll::Pending.
        let _ = self.sender.poll_next_unpin(cx);

        while let Some(msg) = ready!(self.input_stream.poll_next_unpin(cx)) {
            // Don't respond to final level updates.
            if msg.level == self.level_update.level {
                continue;
            }

            let response = (self.level_update.clone(), msg.origin());
            self.sender.send(response);
        }

        Poll::Ready((self.level_update.aggregate.clone(), None))
    }
}
/// Stream implementation for consecutive aggregation events
pub struct Aggregation<
    TId: Debug + Clone + Unpin + Send + 'static,
    P: Protocol<TId>,
    N: Network<Contribution = P::Contribution>,
> {
    next_aggregation:
        Option<BoxFuture<'static, (P::Contribution, Option<NextAggregation<TId, P, N>>)>>,
}

impl<
        TId: Debug + Clone + Unpin + Send + 'static,
        TProtocol: Protocol<TId>,
        TNetwork: Network<Contribution = TProtocol::Contribution> + Send + 'static,
    > Aggregation<TId, TProtocol, TNetwork>
{
    pub fn new(
        protocol: TProtocol,
        config: Config,
        own_contribution: TProtocol::Contribution,
        input_stream: LevelUpdateStream<TProtocol, TId>,
        network: TNetwork,
    ) -> Self {
        // Create the Sender, buffering a single message per recipient.
        let sender = LevelUpdateSender::new(protocol.partitioner().size(), network);

        let next_aggregation =
            NextAggregation::new(protocol, config, own_contribution, input_stream, sender)
                .next()
                .boxed();

        Self {
            next_aggregation: Some(next_aggregation),
        }
    }
}

impl<
        TId: Debug + Clone + Unpin + Send + 'static,
        P: Protocol<TId> + Debug,
        N: Network<Contribution = P::Contribution>,
    > Stream for Aggregation<TId, P, N>
{
    type Item = P::Contribution;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // check if there is a next_aggregation
        let next_aggregation = match self.next_aggregation.as_mut() {
            // If there is Some(next_aggregation) proceed with it
            Some(next_aggregation) => next_aggregation,
            // If there is None that means all signatories have signed the payload so there is nothing more to aggregate after the last returned value.
            // Thus return Poll::Ready(None) as the previous value can no longer be improved.
            None => return Poll::Ready(None),
        };

        // Poll the next_aggregate future. If it is still Poll::Pending return Poll::Pending as well. (hidden within ready!)
        let (aggregate, next_aggregation) = ready!(next_aggregation.poll_unpin(cx));

        self.next_aggregation = next_aggregation.map(|next_aggregation| {
            if next_aggregation.is_complete_aggregate(&aggregate) {
                FinishedAggregation::from(next_aggregation, aggregate.clone()).boxed()
            } else {
                next_aggregation.next().boxed()
            }
        });

        // At this point a new aggregate was returned so the Stream returns it as well.
        Poll::Ready(Some(aggregate))
    }
}
