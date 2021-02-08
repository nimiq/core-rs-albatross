use std::fmt::Debug;
use std::pin::Pin;

use beserial::{Deserialize, Serialize};

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::task::{Context, Poll};
use futures::{ready, select, FutureExt, Sink, Stream, StreamExt};

use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant, Interval};

use crate::config::Config;
use crate::contribution::AggregatableContribution;
use crate::level::Level;
use crate::partitioner::Partitioner;
use crate::identity::{Identity, IdentityRegistry};
use crate::protocol::Protocol;
use crate::store::ContributionStore;
use crate::todo::TodoList;
use crate::update::{LevelUpdate, LevelUpdateMessage};

// TODOS:
// * protocol.store RwLock.
// * level.state RwLock
// * SinkError.
// * Evaluator::new -> threshold (now covered outside of this crate)

#[derive(std::fmt::Debug)]
pub struct SinkError {} // TODO

/// Future implementation for the next aggregation event
struct NextAggregation<P: Protocol, T: Clone + Debug + Eq + Serialize + Deserialize + Sized + Send + Sync + Unpin> {
    /// Handel configuration
    config: Config,

    /// Levels
    levels: Vec<Level>,

    /// Remaining Todos
    todos: Pin<Box<TodoList<P::Contribution, P::Evaluator>>>,

    /// The protocol specifying how this aggregation works.
    protocol: P,

    /// Our contribution
    contribution: P::Contribution,

    /// The tag for this aggregation
    tag: T,

    /// Sink used to relay messages
    sender: UnboundedSender<(LevelUpdateMessage<P::Contribution, T>, usize)>,

    /// Interval for starting the next level regarless of previous levels completion
    start_level_interval: Interval,

    /// Interval for sending level updates to the corresponding peers regardless of progression
    periodic_update_interval: Interval,

    /// the level which needs activation next
    next_level_timeout: usize,
}

impl<P: Protocol, T: Clone + Debug + Eq + Serialize + Deserialize + Sized + Send + Sync + Unpin> NextAggregation<P, T> {
    pub fn new(
        protocol: P,
        tag: T,
        config: Config,
        own_contribution: P::Contribution,
        input_stream: BoxStream<'static, LevelUpdate<P::Contribution>>,
        sender: UnboundedSender<(LevelUpdateMessage<P::Contribution, T>, usize)>,
    ) -> Self {
        // invoke the partitioner to create the level structure of peers.
        let levels: Vec<Level> = Level::create_levels(protocol.partitioner());

        // Create an empty todo list  which can later be polled for the best available todo.
        let todos = Box::pin(TodoList::new(protocol.evaluator(), input_stream));

        // Regarless of level completion consecutive levels need to be activated at some point. Activate Levels every time this interval ticks,
        // if the level has not already been activated due to level completion
        let start_level_interval = interval_at(Instant::now() + config.timeout, config.timeout);

        // Every `config.update_interval` send Level updates to corresponding peers no matter the aggregations progression
        // (makes sure other peers can catch up).
        let periodic_update_interval = interval_at(Instant::now() + config.update_interval, config.update_interval);

        // create the NextAggregation struct
        let this = Self {
            protocol,
            tag,
            config,
            todos,
            levels,
            contribution: own_contribution.clone(),
            sender,
            start_level_interval,
            periodic_update_interval,
            next_level_timeout: 0,
        };

        // make sure the contribution of this instance is added to the store
        this.protocol.store().write().put(own_contribution.clone(), 0, this.protocol.registry().signers_identity(&own_contribution.contributors()));

        // and check if that already completes a level
        // Level 0 always only contains a single signature, the one of this instance. Thus it will always complete that level,
        // startinng the next one and sending updates.
        this.check_completed_level(own_contribution, 0);

        // return the NextAggregation struct
        this
    }

    /// Starts level `level`
    fn start_level(&self, level: usize) {
        let level = self.levels.get(level).unwrap_or_else(|| panic!("Attempted to start invalid level {}", level));
        trace!("Starting level {}: Peers: {:?}", level.id, level.peer_ids);

        level.start();

        if level.id > 0 {
            // get the current best for the level. Freeing the lock as soon as possible to continue working on todos
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

    /// Check if a level was completed TODO: remove contribution parameter as it is not used at all.
    fn check_completed_level(&self, contribution: P::Contribution, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Attempted to check completeness of invalid level: {}", level));

        trace!(
            "Checking for completed level {}: signers={:?}",
            level.id,
            contribution.contributors().iter().collect::<Vec<usize>>()
        );

        // check if level already is completed
        {
            let level_state = level.state.read();
            if level_state.receive_completed {
                // The level was completed before so nothing more to do.
                trace!("check_completed_level: receive_completed=true");
                return;
            }
        }

        // first get the current contributor count for this level. Release the lock as soon as possible
        // to continue working on todos.
        let num_contributors = {
            let store = self.protocol.store();
            let store = store.read();
            match self.protocol.registry().signers_identity(&store
                .best(level.id)
                .unwrap_or_else(|| panic!("Expected a best signature for level {}", level.id))
                .contributors()) {
                    Identity::None => 0,
                    Identity::Single(_) => 1,
                    Identity::Multiple(ids) => ids.len(),
            }
        };

        trace!("level {} - #{}/{}", level.id, num_contributors, level.num_peers());

        // If the number of contributors on this level is equal to the number of peers on this level it is completed.
        if num_contributors == level.num_peers() {
            trace!("Level {} complete", level.id);
            {
                // aquire write lock and set the level state for this level to completed.
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
                // aquire read lock to retrieve the current commbined contribution for the next lower level
                let store = self.protocol.store();
                let store = store.read();
                store.combined(i - 1)
            };

            // if there is an aggregate contribution for given level i send it out to the peers of that level.
            if let Some(multisig) = combined {
                let level = self.levels.get(i).unwrap_or_else(|| panic!("No level {}", i));
                if level.update_signature_to_send(&multisig.clone()) {
                    // XXX Do this without cloning
                    self.send_update(multisig, level, self.config.peer_count);
                }
            }
        }
    }

    /// Send updated `contribution` for `level` to `count` peers
    ///
    /// for incomplete levels the contribution containing soley this nodes contribution is send alongside the aggregate
    fn send_update(&self, contribution: P::Contribution, level: &Level, count: usize) {
        let peer_ids = level.select_next_peers(count);

        // if there are peers to send the update to send them
        if !peer_ids.is_empty() {
            // incomplete levels will always also continue to send their own contribution, alongside the aggreagte.
            let individual = if level.receive_complete() { None } else { Some(self.contribution.clone()) };

            // create the LevelUpdate with the aggregate contribution, the level, our node id and, if the level is incomplete, our own contribution
            let update = LevelUpdate::<P::Contribution>::new(contribution, individual, level.id, self.protocol.node_id());

            // Tag the LevelUpdate with the tag this aggregation runs over creating a LevelUpdateMessage.
            let update_msg = update.with_tag(self.tag.clone());
            for peer_id in peer_ids {
                self.sender
                    // `unbounded_send` is not a future and thus will not block execution.
                    .unbounded_send((update_msg.clone(), peer_id))
                    // If an error occured that means the receiver no longer exists or was closed which should never happen.
                    .expect("Message could not be send to unbounded_channel sender");
            }
        }
    }

    /// Send updates for every level to every peer accordingly.
    fn automatic_update(&mut self) {
        trace!("resending Updates");
        for level in self.levels.iter().skip(1) {
            // Get the current best aggregate from store (no clone() needed as that already happens within the store)
            // freeing the lock as soon as possible for the todo aggregating to continue.
            let aggregate = {
                let store = self.protocol.store();
                let store = store.read();
                store.combined(level.id - 1)
            };
            // For an existing aggregate for this level send it around to the respective peers.
            if let Some(aggregate) = aggregate {
                self.send_update(aggregate, &level, self.config.update_count);
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
                                let best =  {
                                    let store = self.protocol.store();
                                    let store = store.read();

                                    store.combined(last_level.id)
                                };

                                if let Some(best) = best {
                                    if best.num_contributors() == self.protocol.partitioner().size() {
                                        // if there is a best aggregate and this aggregate can no longer be improved upon (all contributors are already present)
                                        // return the aggregate and None to signal no improvments can be made
                                        return (best, None);
                                    } else {
                                        // if the best aggregate can still be improved return the aggregate and self to continue the aggregation
                                        return (best, Some(self));
                                    }
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
}

/// Stream implementation for consecutive aggregation events
pub struct Aggregation<P: Protocol, T: Clone + Debug + Eq + Serialize + Deserialize + Sized + Send + Sync + Unpin> {
    next_aggregation: Option<BoxFuture<'static, (P::Contribution, Option<NextAggregation<P, T>>)>>,
    network_handle: Option<JoinHandle<()>>,
}

impl<P: Protocol, T: Clone + Debug + Eq + Serialize + Deserialize + Sized + Send + Sync + Unpin + 'static> Aggregation<P, T> {
    pub fn new<E: Debug + 'static>(
        protocol: P,
        tag: T,
        config: Config,
        own_contribution: P::Contribution,
        input_stream: BoxStream<'static, LevelUpdate<P::Contribution>>,
        output_sink: Box<(dyn Sink<(LevelUpdateMessage<P::Contribution, T>, usize), Error = E> + Unpin + Send)>,
    ) -> Self {
        // Create an unbounded mpsc channel to buffer network messages for the actual aggregation not having to wait for them to get send.
        // A future optimization could be to have this task not simply forward all messages but filter out those which have become obsolete
        // by subsequent messages.
        let (sender, receiver) = unbounded::<(LevelUpdateMessage<P::Contribution, T>, usize)>();

        // Spawn a task emptying put the receiver of the created mpsc. Note that this task will terminate once all senders are dropped leading
        // to the stream no longer producing any new items. In turn the forward future will resolve terminating the task.
        let network_handle = tokio::spawn(async move {
            if let Err(err) = receiver.map(Ok).forward(output_sink).await {
                warn!("Error sending messages: {:?}", err);
            }
        });

        let next_aggregation = NextAggregation::new(protocol, tag, config, own_contribution, input_stream, sender)
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

impl<P: Protocol + Debug, T: Clone + Debug + Eq + Serialize + Deserialize + Send + Sync + Unpin + 'static> Stream for Aggregation<P, T> {
    type Item = P::Contribution;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // check if there is a next_aggregation
        let next_aggregation = match self.next_aggregation.as_mut() {
            // If there is Some(next_aggregation) proceed with it
            Some(next_aggreagtion) => next_aggreagtion,
            // If there is None that means all signatories have signed the payload so there is nothing more to aggregate after the last returned value.
            // Thus return Poll::Ready(None) as the previous value can no longer be improved.
            None => return Poll::Ready(None),
        };

        // Poll the next_aggregate future. If it is still Poll::Pending return Poll::Pending as well. (hidden within ready!)
        let (aggregate, next_aggregation) = ready!(next_aggregation.poll_unpin(cx));

        self.next_aggregation = match next_aggregation {
            // If there is Some(next_agregation) set its next() future as the next output of this stream
            Some(next_aggregation) => Some(next_aggregation.next().boxed()),
            // If there is None set it as well so next time the sream is polled it will correctly signal that there are no more items coming.
            None => None,
        };
        // At this point a new aggregate was returned so the Stream returns it as well.
        Poll::Ready(Some(aggregate))
    }
}
