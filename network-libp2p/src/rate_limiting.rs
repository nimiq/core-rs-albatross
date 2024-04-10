use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use instant::Instant;
use libp2p::PeerId;

/// Holds the expiration time for a given peer and request type. This struct defines the ordering for the btree set.
/// The smaller expiration times come first.
#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub(crate) struct Expiration {
    pub(crate) peer_id: PeerId,
    pub(crate) req_type: u16,
    pub(crate) expiration_time: Instant,
}

impl Expiration {
    pub(crate) fn new(peer_id: PeerId, req_type: u16, expiration_time: Instant) -> Self {
        Self {
            peer_id,
            req_type,
            expiration_time,
        }
    }
}

impl Ord for Expiration {
    fn cmp(&self, other: &Self) -> Ordering {
        self.expiration_time
            .cmp(&other.expiration_time)
            .then_with(|| self.peer_id.cmp(&other.peer_id))
            .then_with(|| self.req_type.cmp(&other.req_type))
    }
}
impl PartialOrd for Expiration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The structure to be used to store the pending to delete rate limits.
/// We must ensure there are no duplicates, constant complexity while accessing peer and request type and ordering by
/// expiration time.
/// These structs should contain the same information and maintain consistency.
#[derive(Debug, Default)]
pub(crate) struct PendingDeletion {
    /// The hash map of the rate limits.
    by_peer_and_req_type: HashMap<(PeerId, u16), RateLimit>,
    /// The ordered set of rate limits by expiration time, peer id and request type.
    by_expiration_time: BTreeSet<Expiration>,
}

impl PendingDeletion {
    /// Retrieves the first item by expiration.
    pub(crate) fn first(&self) -> Option<&Expiration> {
        self.by_expiration_time.first()
    }

    /// Adds to both structures the new entry. If the entry already exists we replace it on both structs.
    pub(crate) fn insert(&mut self, peer_id: PeerId, req_type: u16, rate_limit: &RateLimit) {
        if let Some(expiration_peer) = self
            .by_peer_and_req_type
            .insert((peer_id, req_type), rate_limit.clone())
        {
            self.by_expiration_time.remove(&Expiration::new(
                peer_id,
                req_type,
                expiration_peer.next_reset_time(),
            ));
        }
        self.by_expiration_time.insert(Expiration::new(
            peer_id,
            req_type,
            rate_limit.next_reset_time(),
        ));
    }

    /// Removes the first item by expiration date, on both structures.
    pub(crate) fn remove_first(&mut self) {
        if let Some(peer_expiration) = self.by_expiration_time.pop_first() {
            assert!(
                self.by_peer_and_req_type
                    .remove(&(peer_expiration.peer_id, peer_expiration.req_type))
                    .is_some(),
                "The pending for deletion rate limits should be consistent among them"
            )
        }
    }
}

/// The structure to be used to limit the number of requests to a limit of allowed_occurrences within a block_range.
#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct RateLimit {
    /// Max allowed requests.
    allowed_occurrences: u32,
    /// The range/window of time.
    time_window: Duration,
    /// The timestamp of the last reset.
    last_reset: Instant,
    /// The counter of requests submitted within the current block range.
    occurrences_counter: u32,
}

impl RateLimit {
    pub fn new(allowed_occurrences: u32, time_window: Duration, last_reset: Instant) -> Self {
        RateLimit {
            allowed_occurrences,
            time_window,
            last_reset,
            occurrences_counter: 0,
        }
    }

    /// Updates the last_reset if needed and then increments the counter of number of requests by
    /// the specified number.
    /// Receives the number to increment the counter and the current time measured in seconds.
    pub fn increment_and_is_allowed(&mut self, request_count: u32) -> bool {
        let current_time = Instant::now();
        if self.next_reset_time() <= current_time {
            self.last_reset = current_time;
            self.occurrences_counter = 0;
        }
        self.occurrences_counter += request_count;
        self.occurrences_counter <= self.allowed_occurrences
    }

    /// Checks if this object can be deleted by understanding if there are still active counters.
    pub fn can_delete(&self, current_time: Instant) -> bool {
        self.occurrences_counter == 0 || self.next_reset_time() <= current_time
    }

    /// Returns the timestamp for the next reset of the counters.
    pub fn next_reset_time(&self) -> Instant {
        self.last_reset + self.time_window
    }
}
