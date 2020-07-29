use async_trait::async_trait;
use std::fmt;
use std::sync::{Arc, Weak};

use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use tokio::stream::StreamExt;

use tokio::time;

use beserial::{Deserialize, Serialize};
use nimiq_network_interface::network::Network;

use crate::config::Config;
use crate::contribution::AggregatableContribution;
use crate::evaluator::Evaluator;
use crate::level::Level;
use crate::protocol::Protocol;
use crate::store::ContributionStore;
use crate::todo::{TodoItem, TodoList};
use crate::update::{LevelUpdate, LevelUpdateMessage};

#[async_trait]
pub trait HandelAggreation<
    P: Protocol,
    N: Network,
    T: Clone + fmt::Debug + Eq + Serialize + Deserialize + Send + Sync + 'static,
>
{
    async fn start(
        tag: T,
        own_contribution: P::Contribution,
        protocol: P,
        config: Config,
        network: Arc<N>,
    ) -> P::Contribution;
}

// TODO remove
struct AggregationState<C: AggregatableContribution> {
    /// Our contribution
    contribution: Option<C>, // todo move into Aggregattion
}

pub struct Aggregation<
    P: Protocol,
    N: Network,
    T: Clone + fmt::Debug + Eq + Serialize + Deserialize + Send + Sync + 'static,
> {
    /// Handel configuration, including the hash being signed, this node's contributed signature, etc.
    config: Config,

    /// Levels
    levels: Vec<Level>,

    /// Signatures that still need to be processed
    // todos: Arc<TodoList<P::Contribution, P::Evaluator>>,

    /// The underlying protocol
    pub protocol: P,

    /// Our contribution behind a RwLock
    contribution: RwLock<AggregationState<P::Contribution>>,

    /// Network for all communication purposes
    network: Arc<N>,

    /// The message this aggregation is about (i.e. block hash)
    tag: T,
}

impl<
        P: Protocol + fmt::Debug,
        N: Network,
        T: Clone + fmt::Debug + Eq + Serialize + Deserialize + Send + Sync + 'static, // TODO think about how to get rid of this generic
    > Aggregation<P, N, T>
{
    fn new(tag: T, protocol: P, config: Config, network: Arc<N>) -> Arc<Self> {
        let levels: Vec<Level> = Level::create_levels(protocol.partitioner());

        // create aggregation
        Arc::new(Self {
            config,
            levels,
            protocol,
            contribution: RwLock::new(AggregationState { contribution: None }),
            tag,
            network,
        })
    }

    /// only exposed function aside from `new`. Starts the aggregation of contributions and returns the bestt contribution after the threshold
    /// is reached and a grace period has passed.
    ///
    /// A ReceiveFromAll for LevelUpdateMessages will be started, overriding existing Consumers for this message type. In case the previoous consumer
    /// for that message type is necessary (i.e. View changes) it needs to be re-established after this aggregation resolves.
    pub async fn start(
        tag: T,
        own_contribution: P::Contribution,
        protocol: P,
        config: Config,
        network: Arc<N>,
    ) -> P::Contribution {
        trace!("Handel started for {:?}", &tag);

        let t = tag.clone();
        // receive all msg then filter them to make sure they have the correct tag. Strip the Wrappers.
        let input_stream = Box::pin(
            network
                .receive_from_all::<LevelUpdateMessage<P::Contribution, T>>()
                .filter(move |msg| msg.0.tag == t)
                .map(move |msg| msg.0.update),
        );
        let mut todos = TodoList::new(protocol.evaluator(), input_stream);

        let this = Self::new(tag.clone(), protocol, config, network);

        // add this nodes own contribution as it will not be received over the network.
        this.push_contribution(own_contribution).await;

        // start sending periodic updates
        let weak = Arc::downgrade(&this);
        let interval = time::interval(this.config.update_interval);
        tokio::spawn(Self::periodic_loop(weak, interval));

        // start starting levels on a schedule
        let weak = Arc::downgrade(&this);
        // This creates a linear schedule. In case anther schedule is beneficial a different Interval must be provided
        let interval = time::interval(this.config.timeout);
        tokio::spawn(Self::init_timeout(weak, interval));

        // continuously work on Todos
        loop {
            if let Some(TodoItem {
                level,
                contribution,
            }) = todos.next().await
            {
                trace!("Processing: level={}: {:?}", level, contribution);

                let result = this.protocol.verify(&contribution).await;
                if result.is_ok() {
                    {
                        let store = this.protocol.store();
                        let mut store = store.write();
                        store.put(contribution.clone(), level);
                    }

                    this.check_completed_level(&contribution, level).await;
                    if this.check_final_contribution() {
                        break;
                    }
                } else {
                    warn!("Invalid signature: {:?}", result);
                }
            } else {
                panic!("TodoItem stream returned Ready(None), which should never happen");
            }
        }

        // return the best available result
        let aggregate = this
            .result()
            .expect("Final Aggregation is not existent even though it should be!");

        trace!("Handel for {:?} cpmpleted: {:?}", &this.tag, &aggregate);

        aggregate
    }

    /// Sends Updates periodically in order for peers who have fallen out of the network and rejoined
    /// to be able to get up to speed again.
    /// Terminates on its own once the handel instance is dropped.
    async fn periodic_loop(this: Weak<Self>, mut interval: time::Interval) {
        while interval.next().await.is_some() {
            if let Some(this) = Weak::upgrade(&this) {
                // for every level send the best available signature again if there is one.
                trace!("resending Updates");
                for level in this.levels.iter().skip(1) {
                    let store = this.protocol.store();
                    let aggregate = store.read().combined(level.id - 1);
                    drop(store);
                    if let Some(aggregate) = aggregate {
                        this.send_update(aggregate, &level, this.config.update_count)
                            .await;
                    }
                }
            } else {
                // If the Weak Pointer cannot be upgraded the Handel instance no longer exists and this Task should
                // terminate as it can no longer do anything.
                break;
            }
        }
    }

    /// The timeout strategy 'activates' levels depending on the time that has passed since the aggregation started.
    /// Terminates n its own once either all levels are completed, or the handel instance is dropped.
    async fn init_timeout(this: Weak<Self>, mut interval: time::Interval) {
        let mut next_level_timeout: usize = 0;
        while interval.next().await.is_some() {
            if let Some(this) = Weak::upgrade(&this) {
                let level = next_level_timeout;
                if level < this.num_levels() {
                    trace!("Timeout for {:?} at level {}", this.protocol, level);
                    next_level_timeout += 1;
                    this.start_level(level).await;
                } else {
                    break; // terminate this function dropping the interval if all levels have been activated
                }
            } else {
                // If the Weak Pointer cannot be upgraded the Handel instance no longer exists and this Task should
                // terminate as it can no longer do anything.
                break;
            }
        }
    }

    /// adds this nodes own signature to the aggregation
    async fn push_contribution(&self, contribution: P::Contribution) {
        // only allow contributions with a single contributor, which also must be this node itself.
        assert!(
            contribution.num_contributors() == 1
                && contribution
                    .contributors()
                    .contains(self.protocol.node_id())
        );
        {
            let state = self.contribution.upgradable_read();
            if state.contribution.is_some() {
                error!("Contribution already exists");
                return;
            }
            let mut state = RwLockUpgradableReadGuard::upgrade(state);
            state.contribution = Some(contribution.clone());
            // drop(state);
        }

        // put our own contribution into store at level 0
        self.protocol.store().write().put(contribution.clone(), 0);

        // check for completed levels
        self.check_completed_level(&contribution, 0).await;
        self.check_final_contribution(); // CHECKME

        // sending of level 0 is done by check_completed level
    }

    pub fn num_levels(&self) -> usize {
        self.levels.len()
    }

    /// Starts level `level`
    async fn start_level(&self, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Timeout for invalid level {}", level));
        trace!("Starting level {}: Peers: {:?}", level.id, level.peer_ids);

        level.start();

        if level.id > 0 {
            let store = self.protocol.store();
            let best = store.read().combined(level.id - 1);
            drop(store);

            if let Some(best) = best {
                self.send_update(best, level, self.config.peer_count).await;
            }
        }
    }

    /// Send updated `contribution` for `level` to `count` peers
    ///
    /// for incomplete levels the contribution containing soley this nodes contribution is send alongside the aggregate
    async fn send_update(&self, contribution: P::Contribution, level: &Level, count: usize) {
        let peer_ids = level.select_next_peers(count);

        if !peer_ids.is_empty() {
            // TODO: optimize, if the multi-sig, only contains our individual, we don't have to send it
            let individual = if level.receive_complete() {
                None
            } else {
                self.contribution.read().contribution.clone()
            };
            // level needs to be in terms of the reciients tree not ours

            let update = LevelUpdate::<P::Contribution>::new(
                contribution,
                individual,
                level.id,
                self.protocol.node_id(),
            );

            let update_msg = update.with_tag(self.tag.clone());

            self.network.broadcast(&update_msg).await;
            //self.network.send_to(peer_ids, &update_msg).await;
        }
    }

    /// Check if a level was completed
    async fn check_completed_level(&self, contribution: &P::Contribution, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level));

        trace!(
            "Checking for completed level {}: signers={:?}",
            level.id,
            contribution.contributors().iter().collect::<Vec<usize>>()
        );

        // check if level is completed
        {
            let level_state = level.state.read();
            if level_state.receive_completed {
                // The level was completed before so nothing more to do.
                trace!("check_completed_level: receive_completed=true");
                return;
            }
        }

        let num_contributors = {
            let store = self.protocol.store();
            let store = store.read();
            store
                .best(level.id)
                .unwrap_or_else(|| panic!("Expected a best signature for level {}", level.id))
                .num_contributors()
        };

        if num_contributors == level.num_peers() {
            trace!("Level {} complete", level.id);
            {
                let mut level_state = level.state.write();
                level_state.receive_completed = true;
            }
            if level.id + 1 < self.levels.len() {
                // activate next level
                self.start_level(level.id + 1).await
            }
        }

        for i in level.id + 1..self.levels.len() {
            let combined = {
                let store = self.protocol.store();
                let store = store.read();
                store.combined(i - 1)
            };

            if let Some(multisig) = combined {
                let level = self
                    .levels
                    .get(i)
                    .unwrap_or_else(|| panic!("No level {}", i));
                if level.update_signature_to_send(&multisig.clone()) {
                    // XXX Do this without cloning
                    self.send_update(multisig, &level, self.config.peer_count)
                        .await;
                }
            }
        }
    }

    /// Check if the best signature is final
    ///
    /// TODO: In some cases we still want to make the final signature "more final", i.e. the
    /// pbft prepare signature still can become better and thus influence the finality of the
    /// commit signature.
    fn check_final_contribution(&self) -> bool {
        let last_level = self.levels.last().expect("No levels");
        let store = self.protocol.store();
        let store = store.read();

        trace!(
            "Checking we have an aggregation above threshold: last_level={}",
            last_level.id
        );

        if let Some(combined) = store.combined(last_level.id) {
            trace!("Best combined signature: {:?}", combined);
            if self.protocol.evaluator().is_final(&combined) {
                // XXX Do this without cloning
                trace!("Best aggregate is above threshold");

                return true;
            }
        }
        false
    }

    /// finds the best aggregation and returns it if it is final.
    pub fn result(&self) -> Option<P::Contribution> {
        let last_level = self.levels.last().expect("No levels");

        let store = self.protocol.store();
        let store = store.read();

        if let Some(combined) = store.combined(last_level.id) {
            if self.protocol.evaluator().is_final(&combined) {
                return Some(combined);
            }
        }
        None
    }
}

#[async_trait]
impl<
        P: Protocol + fmt::Debug,
        N: Network,
        T: Clone + fmt::Debug + Eq + Serialize + Deserialize + Send + Sync + 'static,
    > HandelAggreation<P, N, T> for Aggregation<P, N, T>
{
    async fn start(
        tag: T,
        own_contribution: P::Contribution,
        protocol: P,
        config: Config,
        network: Arc<N>,
    ) -> P::Contribution {
        Self::start(tag, own_contribution, protocol, config, network).await
    }
}
