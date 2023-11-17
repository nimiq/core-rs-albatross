use std::{cmp::min, sync::Arc};

use parking_lot::RwLock;
use rand::{seq::SliceRandom, thread_rng};

use crate::{
    contribution::AggregatableContribution,
    partitioner::{Partitioner, PartitioningError},
};

/// Struct that defines the state of a level
#[derive(Clone, Debug)]
pub struct LevelState {
    /// Send is already started
    pub send_started: bool,
    /// Receive is already completed
    pub receive_completed: bool,
    /// The position of the peer where the next send must go to
    pub send_peers_pos: usize,
    /// The size of the signature to send
    pub send_signature_size: usize,
    /// The number of peers that a send is expected to go to
    pub send_peers_count: usize,
}

/// Struct that defines an Aggregation Level
#[derive(Debug)]
pub struct Level {
    /// The ID of this level
    pub id: usize,
    /// The Peer IDs on this level
    pub peer_ids: Vec<usize>,
    /// The full size of the expected signature of the combined signature of
    /// all levels up to (including) the current one.
    pub send_expected_full_size: usize,
    /// The state of this level
    pub state: RwLock<LevelState>,
}

impl Level {
    /// Creates a new level given its id, the set of peers and the expected
    /// number of peers to consider this level send complete
    pub fn new(id: usize, peer_ids: Vec<usize>, send_expected_full_size: usize) -> Level {
        Level {
            id,
            peer_ids,
            send_expected_full_size,
            state: RwLock::new(LevelState {
                send_started: false,
                receive_completed: false,
                send_peers_pos: 0,
                send_signature_size: 0,
                send_peers_count: 0,
            }),
        }
    }

    /// Returns the number of peers in this level
    pub fn num_peers(&self) -> usize {
        self.peer_ids.len()
    }

    /// Returns whether this level is empty
    pub fn is_empty(&self) -> bool {
        self.peer_ids.len() == 0
    }

    /// Creates a set of levels given a partitioner
    pub fn create_levels<P: Partitioner, TId: std::fmt::Debug>(
        partitioner: Arc<P>,
        id: TId,
    ) -> Vec<Level> {
        let mut levels: Vec<Level> = Vec::new();
        let mut first_active = false;
        let mut send_expected_full_size: usize = 1;
        let mut rng = thread_rng();

        for i in 0..partitioner.levels() {
            match partitioner.range(i) {
                Ok(ids) => {
                    let mut ids = ids.collect::<Vec<usize>>();
                    ids.shuffle(&mut rng);

                    let size = ids.len();
                    trace!(
                        ?id,
                        level = i,
                        peers_on_level = ?ids,
                        "Peers on Level",
                    );
                    let level = Level::new(i, ids, send_expected_full_size);

                    if !first_active {
                        first_active = true;
                        level.state.write().send_started = true;
                    }

                    levels.push(level);
                    send_expected_full_size += size;
                }
                Err(PartitioningError::EmptyLevel { .. }) => {
                    let level = Level::new(i, vec![], send_expected_full_size);
                    levels.push(level);
                }
                Err(e) => panic!("{}", e),
            }
        }

        levels
    }

    /// Returns whether this level is active
    pub fn active(&self) -> bool {
        let state = self.state.read();
        state.send_started && state.send_peers_count < self.peer_ids.len()
    }

    /// Returns whether this level is received complete
    pub fn receive_complete(&self) -> bool {
        let state = self.state.read();
        state.receive_completed
    }

    /// Selects the set of next peers to send an update to for this level given a count of them
    pub fn select_next_peers(&self, count: usize) -> Vec<usize> {
        if self.id == 0 || self.is_empty() {
            vec![]
        } else {
            let size = min(count, self.peer_ids.len());
            let mut selected: Vec<usize> = Vec::new();

            let mut state = self.state.write();
            for _ in 0..size {
                // NOTE: Unwrap is safe, since we make sure at least `size` elements are in `self.peers`
                selected.push(*self.peer_ids.get(state.send_peers_pos).unwrap());
                state.send_peers_pos += 1;
                if state.send_peers_pos >= self.peer_ids.len() {
                    state.send_peers_pos = 0;
                }
            }

            selected
        }
    }

    /// Updates the signature to send
    pub fn update_signature_to_send<C: AggregatableContribution>(&self, signature: &C) -> bool {
        let mut state = self.state.write();

        if state.send_signature_size >= signature.num_contributors() {
            return false;
        }

        state.send_signature_size = signature.num_contributors();
        state.send_peers_count = 0;

        if state.send_signature_size == self.send_expected_full_size {
            state.send_started = true;
            return true;
        }

        false
    }

    /// Starts the level if not already started.
    ///
    /// If the level was started before returns false, otherwise returns true.
    pub fn start(&self) -> bool {
        let mut state = self.state.write();
        if state.send_started {
            false
        } else {
            state.send_started = true;
            true
        }
    }
}

#[cfg(test)]
mod test {
    use nimiq_collections::bitset::BitSet;
    use nimiq_test_log::test;
    use rand::Rng;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::contribution::ContributionError;

    /// Dump Aggregate adding numbers.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Contribution {
        contributors: BitSet,
    }

    impl AggregatableContribution for Contribution {
        fn contributors(&self) -> BitSet {
            self.contributors.clone()
        }

        fn combine(&mut self, _other_contribution: &Self) -> Result<(), ContributionError> {
            unimplemented!()
        }
    }

    #[test]
    fn it_can_handle_empty_level() {
        let mut rng = thread_rng();
        let id: usize = rng.gen_range(0..10);
        let send_expected_full_size = id + rng.gen_range(0..512);
        let level = Level::new(id, [].to_vec(), send_expected_full_size);

        // Check that the level is actually empty
        assert!(level.is_empty());
        assert_eq!(level.num_peers(), 0);

        // Start the level
        level.start();
        assert!(level.state.read().send_started);

        // An empty level can't be active since there is no peer to send updates to
        assert!(!level.active());

        // Check that it can properly select next peers (return empty vector)
        assert!(level.select_next_peers(rng.gen_range(0..512)).is_empty());
    }

    #[test]
    fn it_can_handle_non_empty_level() {
        let mut rng = thread_rng();
        let id: usize = rng.gen_range(0..10);
        let num_ids = rng.gen_range(1..512);
        let mut ids: Vec<usize> = (0..num_ids).map(|_| rng.gen_range(0..=10)).collect();
        let send_expected_full_size = num_ids + rng.gen_range(0..512);
        let level = Level::new(id, ids.clone(), send_expected_full_size);

        // Check that the level is not empty
        assert!(!level.is_empty());
        assert_eq!(level.num_peers(), num_ids);

        // Start the level
        level.start();
        assert!(level.active());

        // Check that it can properly select next peers (return empty vector)
        let select_size = rng.gen_range(0..num_ids) + 1;
        let iterations = num_ids / select_size;
        if id != 0 {
            for _ in 0..iterations {
                let exp_next_peers: Vec<usize> = ids.drain(0..select_size).collect();
                let next_peers = level.select_next_peers(select_size);
                log::debug!(select_size, ?exp_next_peers, ?next_peers);
                assert_eq!(next_peers, exp_next_peers);
            }
        } else {
            // For level 0 the next peers is always empty since it should be our own ID
            assert!(level.select_next_peers(select_size).is_empty());
        }
    }

    #[test]
    fn it_updates_signature_to_send() {
        let mut rng = thread_rng();
        let id: usize = rng.gen_range(0..10);
        let num_ids = rng.gen_range(1..512);
        let ids: Vec<usize> = (0..num_ids).map(|_| rng.gen_range(0..=10)).collect();
        let send_expected_full_size = num_ids + rng.gen_range(0..512);
        let level = Level::new(id, ids.clone(), send_expected_full_size);

        // Create a small contribution
        let mut contributors = BitSet::new();
        contributors.insert(1);
        let contribution = Contribution { contributors };
        assert!(!level.update_signature_to_send(&contribution)); // `state.send_signature_size` should be 1 now

        // Add another contribution
        let mut contributors = BitSet::new();
        contributors.insert(3);
        let contribution = Contribution { contributors };
        assert!(!level.update_signature_to_send(&contribution)); // `state.send_signature_size` should be 2 now

        // Add a smaller contribution
        let mut contributors = BitSet::new();
        contributors.insert(2);
        let contribution = Contribution { contributors };
        assert!(!level.update_signature_to_send(&contribution)); // `state.send_signature_size` should be still 2

        // Now complete the `send_expected_full_size`
        let mut contributors = BitSet::new();
        for i in 0..send_expected_full_size {
            contributors.insert(i);
        }
        let contribution = Contribution { contributors };
        // After the next call, `state.send_signature_size` should be `num_ids` or `contributors.len()` and now it should return `true`
        assert!(level.update_signature_to_send(&contribution));
    }
}
