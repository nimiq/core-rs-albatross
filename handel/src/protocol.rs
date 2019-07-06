use std::sync::Arc;
use std::io::Error as IoError;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::RwLock;
use futures::{future, Future};

use crate::identity::{IdentityRegistry, WeightRegistry};
use crate::verifier::Verifier;
use crate::timeout::TimeoutStrategy;
use crate::store::SignatureStore;
use crate::level::Level;
use crate::evaluator::Evaluator;
use crate::multisig::{MultiSignature, IndividualSignature, Signature};
use crate::partitioner::Partitioner;
use crate::config::Config;
use crate::todo::TodoList;



pub trait Protocol {
    type Registry: IdentityRegistry;
    type Verifier: Verifier;
    type Timeouts: TimeoutStrategy;
    type Store: SignatureStore;
    type Evaluator: Evaluator;
    type Partitioner: Partitioner;

    fn registry(&self) -> Arc<Self::Registry>;
    fn verifier(&self) -> Arc<Self::Verifier>;
    fn timeouts(&self) -> Arc<Self::Timeouts>;
    fn store(&self) -> Arc<RwLock<Self::Store>>;
    fn evaluator(&self) -> Arc<Self::Evaluator>;
    fn partitioner(&self) -> Arc<Self::Partitioner>;

    fn send_to(&self, to: usize, multisig: MultiSignature, individual: Option<IndividualSignature>, level: usize) -> Result<(), IoError>;
}


pub struct Aggregation<P: Protocol> {
    /// Handel configuration, including the hash being signed, this node's contributed signature, etc.
    config: Config,

    /// The aggregation protocol, including identities, signature verification, timeout strategy,
    /// signature store, signature evaluation and network partiitioning
    protocol: P,

    /// This nodes own individual signature
    contribution: IndividualSignature,

    /// Levels
    levels: Vec<Level>,

    /// Signatures that still need to be processed
    todos: TodoList<P::Evaluator>,

    /// Whether the aggregation is finished
    done: AtomicBool,
}


impl<P: Protocol> Aggregation<P> {
    pub fn new(protocol: P, config: Config, contribution: IndividualSignature) -> Self {
        let levels = Level::create_levels(protocol.partitioner());
        let todos = TodoList::new(protocol.evaluator());
        Self {
            config,
            protocol,
            contribution,
            levels,
            todos,
            done: AtomicBool::new(false),
        }
    }

    /// Returns the ID of this node
    ///
    /// NOTE: The ID of the running node is determined by the signer of the node's contribution
    pub fn node_id(&self) -> usize {
        self.contribution.signer
    }

    /// Starts level `level`
    fn start_level(&self, level: usize) {
        debug!("Starting level {}", level);

        let level = self.levels.get(level)
            .unwrap_or_else(|| panic!("Timeout for invalid level {}", level));

        level.start();
        if level.id > 0 {
            if let Some(best) = self.protocol.store().read().combined(level.id - 1) {
                self.send_update(best, level, self.config.peer_count);
            }
        }
    }

    /// Send updated `multisig` for `level` to `count` peers
    fn send_update(&self, multisig: MultiSignature, level: &Level, count: usize) {
        let peer_ids = level.select_next_peers(count);

        let individual = if level.receive_complete() { None } else { Some(self.contribution.clone()) };

        for peer_id in peer_ids {
            self.protocol.send_to(peer_id, multisig.clone(), individual.clone(), level.id)
                .unwrap_or_else(|e| error!("Failed to send update message."))
        }
    }

    fn on_level_timeout(&self, level: usize) {
        self.start_level(level);
    }

    /// Periodic update:
    ///  - check if timeout for level is reached. NOTE: This is done with `on_level_timeout`
    ///  - send a new packet ???
    fn on_update(&self) {
        let store = self.protocol.store();
        let store = store.read();

        // NOTE: Skip level 0
        for level in self.levels.iter().skip(1) {
            // send update
            if let Some(multisig) = store.combined(level.id - 1) {
                self.send_update(multisig, &level, self.config.update_count);
            }
        }
    }

    /// Check if a level was completed
    fn check_completed_level(&self, signature: Signature, level: usize) {
        let level = self.levels.get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level));

        debug!("Checking for completed level {}: signers={:?}", level.id, signature.signers().collect::<Vec<usize>>());

        let store = self.protocol.store();
        let store = store.upgradable_read();

        // check if level is completed
        {
            let mut level_state = level.state.write();

            if level_state.receive_completed {
                debug!("check_completed_level: receive_completed=true");
                return
            }

            let best = store.best(level.id)
                .unwrap_or_else(|| panic!("Expected a best signature for level {}", level.id));

            debug!("check_completed_level: level={}, best.len={}, num_peers={}", level.id, best.len(), level.num_peers());
            if best.len() == level.num_peers() {
                //info!("Level {} complete", level.id);
                level_state.receive_completed = true;

                if level.id + 1 < self.levels.len() {
                    // activate next level
                    self.start_level(level.id + 1)
                }
            }
        }

        for i in level.id + 1 .. self.levels.len() {
            if let Some(multisig) = store.combined(i - 1) {
                let level = self.levels.get(i)
                    .unwrap_or_else(|| panic!("No level {}", i));
                if level.update_signature_to_send(&multisig.clone().into()) { // XXX Do this without cloning
                    self.send_update(multisig, &level, self.config.peer_count);
                }
            }
        }
    }

    /// Check if the best signature is final
    fn check_final_signature(&self, _signature: Signature, _level: usize) {
        let last_level = self.levels.last().expect("No levels");
        let store = self.protocol.store();
        let store = store.read();

        if let Some(combined) = store.combined(last_level.id) {
            if self.protocol.evaluator().is_final(&combined.clone().into()) { // XXX Do this without cloning
                debug!("Last level combined: {:#?}", combined);

                // set done to true
                self.done.store(true, Ordering::Relaxed);

                // TODO: Store final signature?
            }
        }
    }

    pub fn push(&self, origin: usize, multisig: MultiSignature, individual: IndividualSignature, level: usize) {
        let individual_fut = self.protocol.verifier().verify(&Signature::Individual(individual.clone()))
            .map_err(|e| warn!("Invalid individual signature: {}", e));
        let multisig_fut = self.protocol.verifier().verify(&Signature::Multi(multisig.clone()))
            .map_err(|e| warn!("Invalid multisig: {}", e));

        // TODO: Use a `weak_self` here?
        individual_fut.join(multisig_fut)
            .and_then(move |_| {
                info!("TODO: Process signature:");
                info!("origin = {}", origin);
                info!("level = {}", level);
                info!("multisig = {:?}", multisig);
                info!("individual = {:?}", individual);
                Ok(())
            }).wait().expect("Future failed unexpectly");
    }
}
