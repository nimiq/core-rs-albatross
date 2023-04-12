use std::cmp::min;
use std::sync::Arc;

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
    /// The full size of the expected signature of this level
    pub send_expected_full_size: usize,
    /// The state of this level
    pub state: RwLock<LevelState>,
}

impl Level {
    /// Creates a new level given its id, the set of peers and the expected number of peers to consider this level send complete
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

    /// Creates a set of levels given a partitioner
    pub fn create_levels<P: Partitioner>(partitioner: Arc<P>) -> Vec<Level> {
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
                    trace!("Level {} peers: {:?}", i, ids);
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
        if self.id == 0 {
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
