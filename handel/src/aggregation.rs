use std::fmt;
use std::sync::{Arc, Weak};

use futures::{future, Future};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use macros::upgrade_weak;
use utils::mutable_once::MutableOnce;
use utils::observer::PassThroughNotifier;
use utils::timers::Timers;

use crate::config::Config;
use crate::contribution::AggregatableContribution;
use crate::evaluator::Evaluator;
use crate::level::Level;
use crate::protocol::Protocol;
use crate::sender::Sender;
use crate::store::ContributionStore;
use crate::todo::TodoList;
use crate::update::LevelUpdate;

#[derive(Clone, Debug)]
pub enum AggregationEvent<C: AggregatableContribution> {
    Complete { best: C },
    //LevelComplete { level: usize },
    //Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum AggregationTimer {
    Timeout,
    Update,
}

struct AggregationState<C: AggregatableContribution> {
    result: Option<C>,

    /// Our contribution
    contribution: Option<C>,

    next_level_timeout: usize,
}

pub struct Aggregation<P: Protocol> {
    /// Handel configuration, including the hash being signed, this node's contributed signature, etc.
    config: Config,

    /// Levels
    levels: Vec<Level>,

    /// Signatures that still need to be processed
    todos: Arc<TodoList<P::Contribution, P::Evaluator>>,

    /// The underlying protocol
    pub protocol: P,

    /// Timers for updates and level timeouts
    timers: Timers<AggregationTimer>,

    /// Internal state
    state: RwLock<AggregationState<P::Contribution>>,

    /// Weak reference to the Aggregation itself
    self_weak: MutableOnce<Weak<Self>>,

    /// Notifications for completion
    pub notifier: RwLock<PassThroughNotifier<'static, AggregationEvent<P::Contribution>>>,
}

impl<P: Protocol + fmt::Debug> Aggregation<P> {
    pub fn new(protocol: P, config: Config) -> Arc<Self> {
        let levels = Level::create_levels(protocol.partitioner());
        let todos = Arc::new(TodoList::new(protocol.evaluator()));

        // create aggregation
        let this = Arc::new(Self {
            config,
            levels,
            todos,
            protocol,
            timers: Timers::new(),
            state: RwLock::new(AggregationState::<P::Contribution> {
                result: None,
                next_level_timeout: 0,
                contribution: None,
            }),
            self_weak: MutableOnce::new(Weak::new()),
            notifier: RwLock::new(PassThroughNotifier::new()),
        });

        Self::init_background(&this);

        this
    }

    /// adds this nodes own signature to the aggregation
    pub fn push_contribution(&self, contribution: P::Contribution) {
        // only allow contributions with a single contributor, which also must be this node itself.
        assert!(
            contribution.num_contributors() == 1
                && contribution
                    .contributors()
                    .contains(self.protocol.node_id())
        );

        let mut state = self.state.write();
        if state.contribution.is_none() {
            state.contribution = Some(contribution.clone());
            // let signature = Signature::Individual(contribution.clone());

            // drop state before sending updates
            // Drop state before store is locked. Otherwise we have a circular dependency with
            // `send_update` waiting for the state lock, but holding the store lock. See Issue #58
            drop(state);

            // put our own contribution into store at level 0
            self.protocol.store().write().put(contribution.clone(), 0);

            // check for completed levels
            self.check_completed_level(&contribution, 0);
            self.check_final_contribution();

        // send level 0
        // This will be done by check_completed level
        //let level = self.levels.get(0).expect("Level 0 missing");
        //self.send_update(contribution.as_multisig(), level, self.config.peer_count);
        } else {
            error!("Contribution already exists");
        }
    }

    fn init_background(this: &Arc<Self>) {
        unsafe { this.self_weak.replace(Arc::downgrade(&this)) };

        // register timer for updates
        let weak = Arc::downgrade(this);
        this.timers.set_interval(
            AggregationTimer::Update,
            move || {
                let this = upgrade_weak!(weak);
                trace!("Update for {:?}", this.protocol);
                let store = this.protocol.store();
                let store = store.read();
                // NOTE: Skip level 0
                for level in this.levels.iter().skip(1) {
                    // send update
                    if let Some(multisig) = store.combined(level.id - 1) {
                        this.send_update(multisig, &level, this.config.update_count);
                    }
                }
            },
            this.config.update_interval,
        );

        // register timer for level timeouts
        // TODO: This ignores the timeout strategy
        let weak = Arc::downgrade(this);
        this.timers.set_interval(
            AggregationTimer::Timeout,
            move || {
                let this = upgrade_weak!(weak);
                let mut state = this.state.write();
                let level = state.next_level_timeout;
                if level < this.num_levels() {
                    trace!("Timeout for {:?} at level {}", this.protocol, level);
                    state.next_level_timeout += 1;
                    drop(state);
                    this.start_level(level);
                } else {
                    this.timers.clear_interval(&AggregationTimer::Timeout);
                }
            },
            this.config.timeout,
        );

        // spawn thread handling TODOs
        //tokio::spawn(Arc::clone(&this.todos).into_future());
    }

    pub fn num_levels(&self) -> usize {
        self.levels.len()
    }

    /// Starts level `level`
    fn start_level(&self, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Timeout for invalid level {}", level));

        trace!("Starting level {}: Peers: {:?}", level.id, level.peer_ids);

        level.start();
        if level.id > 0 {
            if let Some(best) = self.protocol.store().read().combined(level.id - 1) {
                self.send_update(best, level, self.config.peer_count);
            }
        }
    }

    /// Send updated `multisig` for `level` to `count` peers
    fn send_update(&self, multisig: P::Contribution, level: &Level, count: usize) {
        let peer_ids = level.select_next_peers(count);

        if !peer_ids.is_empty() {
            // TODO: optimize, if the multi-sig, only contains our individual, we don't have to send it
            let individual = if level.receive_complete() {
                None
            } else {
                self.state.read().contribution.clone()
            };
            let update = LevelUpdate::<P::Contribution>::new(
                multisig,
                individual,
                level.id,
                self.protocol.node_id(),
            );

            for peer_id in peer_ids {
                assert_ne!(
                    peer_id,
                    self.protocol.node_id(),
                    "Nodes must not send updates to them-self"
                );
                self.protocol.sender().send_to(peer_id, update.clone());
            }
        }
    }

    /// Check if a level was completed
    fn check_completed_level(&self, contribution: &P::Contribution, level: usize) {
        let level = self
            .levels
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level));

        trace!(
            "Checking for completed level {}: signers={:?}",
            level.id,
            contribution.contributors().iter().collect::<Vec<usize>>()
        );

        let store = self.protocol.store();
        let store = store.upgradable_read();

        // check if level is completed
        {
            let mut level_state = level.state.write();

            if level_state.receive_completed {
                trace!("check_completed_level: receive_completed=true");
                return;
            }

            let best = store
                .best(level.id)
                .unwrap_or_else(|| panic!("Expected a best signature for level {}", level.id));

            trace!(
                "check_completed_level: level={}, best.len={}, num_peers={}",
                level.id,
                best.num_contributors(),
                level.num_peers()
            );
            if best.num_contributors() == level.num_peers() {
                trace!("Level {} complete", level.id);
                level_state.receive_completed = true;

                if level.id + 1 < self.levels.len() {
                    // activate next level
                    self.start_level(level.id + 1)
                }
            }
        }

        for i in level.id + 1..self.levels.len() {
            if let Some(multisig) = store.combined(i - 1) {
                let level = self
                    .levels
                    .get(i)
                    .unwrap_or_else(|| panic!("No level {}", i));
                if level.update_signature_to_send(&multisig.clone()) {
                    // XXX Do this without cloning
                    self.send_update(multisig, &level, self.config.peer_count);
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
        // first check if we're already done
        let state = self.state.upgradable_read();
        if state.result.is_some() {
            return true;
        }

        let last_level = self.levels.last().expect("No levels");
        let store = self.protocol.store();
        let store = store.read();

        trace!(
            "Checking we have a final signature: last_level={}",
            last_level.id
        );

        if let Some(combined) = store.combined(last_level.id) {
            trace!("Best combined signature: {:?}", combined);
            if self.protocol.evaluator().is_final(&combined.clone().into()) {
                // XXX Do this without cloning
                trace!("Best signature is final");

                let mut state = RwLockUpgradableReadGuard::upgrade(state);
                state.result = Some(combined.clone());

                // drop state and store before notify
                drop(state);
                drop(store);

                self.notifier
                    .read()
                    .notify(AggregationEvent::Complete { best: combined });

                return true;
            }
        }
        false
    }

    pub fn result(&self) -> Option<P::Contribution> {
        self.state.read().result.clone()
    }

    pub fn push_update(&self, update: LevelUpdate<P::Contribution>) {
        if self.state.read().result.is_some() {
            // NOP, if we already have a valid multi-signature
            return;
        }

        let LevelUpdate {
            origin,
            level,
            aggregate,
            individual,
        } = update;

        trace!(
            "Level Update: origin={}, level={}, has_individual={}",
            origin,
            level,
            individual.is_some()
        );

        // Future that verifies the individual signature and puts it into the TODO list
        // NOTE: We use `map` instead of `and_then`, because `and_then` needs to return a future,
        //       and `upgrade_weak!` might return `()`.
        let individual_fut = if let Some(individual) = individual {
            // let sig = Signature::Individual(individual);
            let weak = self.self_weak.clone();
            future::Either::A(self.protocol.verify(&individual).map(move |result| {
                if result.is_ok() {
                    let this = upgrade_weak!(weak);
                    this.todos.put(individual, level as usize);
                } else {
                    warn!("Invalid signature: {:?}", result);
                }
            }))
        } else {
            future::Either::B(future::ok::<(), ()>(()))
        };

        // Future that verifies the multi-signature and puts it into the TODO list
        let multisig_fut = {
            // let sig = Signature::Multi(multisig);
            let weak = Weak::clone(&self.self_weak);
            self.protocol.verify(&aggregate).map(move |result| {
                if result.is_ok() {
                    let this = upgrade_weak!(weak);
                    this.todos.put(aggregate, level as usize);
                } else {
                    warn!("Invalid signature: {:?}", result);
                }
            })
        };

        // Creates a future that will first verify the signatures and then gets all good TODOs
        // and applies them.
        // TODO: The processing should not be done in this spawn. It should run in one seperate Spawn
        //       that processes the whole TODO list until completion.
        let process_fut = {
            let weak = Weak::clone(&self.self_weak);
            individual_fut
                .join(multisig_fut)
                .map(move |_| {
                    // continuously put best todo into store, until there is no good one anymore
                    let this = upgrade_weak!(weak);

                    // get store and acquire write lock
                    let store = this.protocol.store();

                    while let Some((signature, level, score)) = this.todos.get_best() {
                        trace!(
                            "Processing: score={}, level={}: {:?}",
                            score,
                            level,
                            signature
                        );

                        // TODO: put signature from todo into store - is this correct?
                        let mut store = store.write();
                        store.put(signature.clone(), level);
                        // drop store before we check for completed levels
                        drop(store);

                        this.check_completed_level(&signature, level);
                        this.check_final_contribution();
                    }
                })
                .map_err(|e| {
                    // Technically nothing here can fail, but we need to handle that case anyway
                    warn!("The signature processing future somehow failed: {:?}", e);
                    e
                })
        };

        tokio::spawn(process_fut);
    }
}
