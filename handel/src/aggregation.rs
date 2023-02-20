use std::fmt::Debug;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{
    future::{BoxFuture, Future},
    ready, select,
    stream::BoxStream,
    FutureExt, Sink, Stream, StreamExt,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant};
use tokio_stream::wrappers::{IntervalStream, UnboundedReceiverStream};

use crate::config::Config;
use crate::contribution::AggregatableContribution;
use crate::identity::IdentityRegistry;
use crate::level::Level;
use crate::partitioner::Partitioner;
use crate::protocol::Protocol;
use crate::store::ContributionStore;
use crate::todo::TodoList;
use crate::update::LevelUpdate;

// TODOS:
// * protocol.store RwLock.
// * level.state RwLock
// * SinkError.
// * Evaluator::new -> threshold (now covered outside of this crate)

type LevelUpdateStream<P> = BoxStream<'static, LevelUpdate<<P as Protocol>::Contribution>>;
type LevelUpdateSender<P> = UnboundedSender<(LevelUpdate<<P as Protocol>::Contribution>, usize)>;

#[derive(std::fmt::Debug)]
pub struct SinkError {} // TODO

/// Future implementation for the next aggregation event
struct NextAggregation<P: Protocol> {
    /// Handel configuration
    config: Config,

    /// Levels
    levels: Vec<Level>,

    /// Remaining Todos
    todos: TodoList<P::Contribution, P::Evaluator>,

    /// The protocol specifying how this aggregation works.
    protocol: P,

    /// Our contribution
    contribution: P::Contribution,

    /// Sink used to relay messages
    sender: LevelUpdateSender<P>,

    /// Interval for starting the next level regardless of previous levels completion
    start_level_interval: IntervalStream,

    /// Interval for sending level updates to the corresponding peers regardless of progression
    periodic_update_interval: IntervalStream,

    /// the level which needs activation next
    next_level_timeout: usize,
}

impl<P: Protocol> NextAggregation<P> {
    pub fn new(
        protocol: P,
        config: Config,
        own_contribution: P::Contribution,
        input_stream: LevelUpdateStream<P>,
        sender: LevelUpdateSender<P>,
    ) -> Self {
        // Invoke the partitioner to create the level structure of peers.
        let levels: Vec<Level> = Level::create_levels(protocol.partitioner());

        // Create an empty todo list which can later be polled for the best available todo.
        let mut todos = TodoList::new(protocol.evaluator(), input_stream);

        // Add our own contribution to the todo list.
        todos.add_contribution(own_contribution.clone(), 0);

        // Regardless of level completion consecutive levels need to be activated at some point. Activate Levels every time this interval ticks,
        // if the level has not already been activated due to level completion
        let start_level_interval =
            IntervalStream::new(interval_at(Instant::now() + config.timeout, config.timeout));

        // Every `config.update_interval` send Level updates to corresponding peers no matter the aggregations progression
        // (makes sure other peers can catch up).
        let periodic_update_interval = IntervalStream::new(interval_at(
            Instant::now() + config.update_interval,
            config.update_interval,
        ));

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
    fn start_level(&self, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Attempted to start invalid level {level}"));
        trace!("Starting level {}: Peers: {:?}", level.id, level.peer_ids);

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
                    self.send_update(best, level, self.config.peer_count);
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
    /// TODO: remove contribution parameter as it is not used at all.
    fn check_completed_level(&self, _contribution: P::Contribution, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Attempted to check completeness of invalid level: {level}"));

        // check if level already is completed
        {
            let level_state = level.state.read();
            if level_state.receive_completed {
                // The level was completed before so nothing more to do.
                return;
            }
        }

        // first get the current contributor count for this level. Release the lock as soon as possible
        // to continue working on todos.
        let num_contributors = {
            let store = self.protocol.store();
            let store = store.read();
            let best = store
                .best(level.id)
                .unwrap_or_else(|| panic!("Expected a best signature for level {}", level.id));
            self.num_contributors(best)
        };

        // If the number of contributors on this level is equal to the number of peers on this level it is completed.
        if num_contributors == level.num_peers() {
            trace!("Level {} complete", level.id);
            {
                // Acquire write lock and set the level state for this level to completed.
                let mut level_state = level.state.write();
                level_state.receive_completed = true;
            }
            // if there is a level with a higher id than the completed one it needs to be activated.
            if level.id + 1 < self.levels.len() {
                // activate next level
                self.start_level(level.id + 1);
            }
        }

        // In order to send updated messages iterate all levels higher than the given level.
        let level_count = self.levels.len();
        for i in level.id + 1..level_count {
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
                    // XXX Do this without cloning
                    self.send_update(multisig, level, self.config.peer_count);
                }
            }
        }
    }

    /// Send updated `contribution` for `level` to `count` peers
    ///
    /// for incomplete levels the contribution containing solely this nodes contribution is sent alongside the aggregate
    fn send_update(&self, contribution: P::Contribution, level: &Level, count: usize) {
        let peer_ids = level.select_next_peers(count);

        // if there are peers to send the update to send them
        if !peer_ids.is_empty() {
            // incomplete levels will always also continue to send their own contribution, alongside the aggregate.
            let individual = if level.receive_complete() {
                None
            } else {
                Some(self.contribution.clone())
            };

            // create the LevelUpdate with the aggregate contribution, the level, our node id and, if the level is incomplete, our own contribution
            let update = LevelUpdate::<P::Contribution>::new(
                contribution,
                individual,
                level.id,
                self.protocol.node_id(),
            );

            // Tag the LevelUpdate with the tag this aggregation runs over creating a LevelUpdateMessage.
            for peer_id in peer_ids {
                if peer_id < self.protocol.partitioner().size() {
                    self.sender
                        // `send` is not a future and thus will not block execution.
                        .send((update.clone(), peer_id))
                        // If an error occurred that means the receiver no longer exists or was closed which should never happen.
                        .expect("Message could not be send to unbounded_channel sender");
                }
            }
        }
    }

    /// Send updates for every level to every peer accordingly.
    fn automatic_update(&mut self) {
        for level in self.levels.iter().skip(1) {
            if level.active() {
                // Get the current best aggregate from store (no clone() needed as that already happens within the store)
                // freeing the lock as soon as possible for the todo aggregating to continue.
                let aggregate = {
                    let store = self.protocol.store();
                    let store = store.read();
                    store.combined(level.id - 1)
                };
                // For an existing aggregate for this level send it around to the respective peers.
                if let Some(aggregate) = aggregate {
                    self.send_update(aggregate, level, self.config.update_count);
                }
            }
        }
    }

    /// activate the next level which needs to be activated.
    fn activate_next_level(&mut self) {
        // the next level which needs activating on timeout.
        let level = self.next_level_timeout;
        // make sure such level exists
        if level < self.levels.len() {
            trace!("Timeout at level {}", level);

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
                item = self.todos.next().fuse() => {
                    match item {
                        Some(todo) => {
                            // verify the contribution
                            let result = self.protocol.verify(&todo.contribution).await;

                            if result.is_ok() {
                                if todo.level == self.protocol.partitioner().levels() {
                                    return (todo.contribution, Some(self));
                                }

                                // if the contribution is valid push it to the store, creating a new aggregate
                                {
                                    let store = self.protocol.store();
                                    let mut store = store.write();
                                    store.put(todo.contribution.clone(), todo.level, self.protocol.registry().signers_identity(&todo.contribution.contributors()));
                                }

                                // in case the level of this todo has not started, start it now as we have already contributions on it.
                                self.start_level(todo.level);
                                // check if a level was completed by the addition of the contribution
                                self.check_completed_level(todo.contribution.clone(), todo.level);

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
                                warn!("Invalid signature: {:?}", result);
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

    fn into_inner(self) -> (LevelUpdateStream<P>, LevelUpdateSender<P>) {
        (self.todos.into_stream(), self.sender)
    }
}

struct FinishedAggregation<P: Protocol> {
    level_update: LevelUpdate<P::Contribution>,
    input_stream: LevelUpdateStream<P>,
    sender: LevelUpdateSender<P>,
}

impl<P: Protocol> FinishedAggregation<P> {
    fn from(aggregation: NextAggregation<P>, aggregate: P::Contribution) -> Self {
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

impl<P: Protocol> Future for FinishedAggregation<P> {
    type Output = (P::Contribution, Option<NextAggregation<P>>);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some(msg) = ready!(self.input_stream.poll_next_unpin(cx)) {
            // Don't respond to final level updates.
            if msg.level == self.level_update.level {
                continue;
            }

            let response = (self.level_update.clone(), msg.origin());
            if let Err(e) = self.sender.send(response) {
                log::warn!(
                    "Failed to send final LevelUpdate to {}: {:?}",
                    msg.origin(),
                    e
                );
            }
        }

        Poll::Ready((self.level_update.aggregate.clone(), None))
    }
}

/// Stream implementation for consecutive aggregation events
pub struct Aggregation<P: Protocol> {
    next_aggregation: Option<BoxFuture<'static, (P::Contribution, Option<NextAggregation<P>>)>>,
    network_handle: Option<JoinHandle<()>>,
}

impl<P: Protocol> Aggregation<P> {
    pub fn new<E: Debug + 'static>(
        protocol: P,
        config: Config,
        own_contribution: P::Contribution,
        input_stream: LevelUpdateStream<P>,
        output_sink: Pin<
            Box<(dyn Sink<(LevelUpdate<P::Contribution>, usize), Error = E> + Unpin + Send)>,
        >,
    ) -> Self {
        // Create an unbounded mpsc channel to buffer network messages for the actual aggregation not having to wait for them to get send.
        // A future optimization could be to have this task not simply forward all messages but filter out those which have become obsolete
        // by subsequent messages.
        let (sender, receiver) = unbounded_channel::<(LevelUpdate<P::Contribution>, usize)>();

        // Spawn a task emptying put the receiver of the created mpsc. Note that this task will terminate once all senders are dropped leading
        // to the stream no longer producing any new items. In turn the forward future will resolve terminating the task.
        let network_handle = tokio::spawn(async move {
            if let Err(err) = UnboundedReceiverStream::new(receiver)
                .map(Ok)
                .forward(output_sink)
                .await
            {
                warn!("Error sending messages: {:?}", err);
            }
        });

        let next_aggregation =
            NextAggregation::new(protocol, config, own_contribution, input_stream, sender)
                .next()
                .boxed();

        Self {
            next_aggregation: Some(next_aggregation),
            network_handle: Some(network_handle),
        }
    }

    pub async fn shutdown(&mut self) {
        // Drop the next aggregation on shutdown.
        // That also drops the sender of the unbounded channel leaving the receiver to consume remaining items and then
        // receiving None as there is no sender left, thus terminating the stream.
        self.next_aggregation = None;
        if let Some(handle) = self.network_handle.take() {
            // Wait for the forward spawn to conclude operation.
            handle.await.expect("network_task returned JoinError")
        }
    }
}

impl<P: Protocol + Debug> Stream for Aggregation<P> {
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
