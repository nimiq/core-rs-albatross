use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use instant::Instant;
use libp2p::{request_response::InboundRequestId, PeerId};
use nimiq_network_interface::request::{RequestCommon, RequestType};
use parking_lot::Mutex;

/// The rate limiting request metadata that will be passed on between the network and the swarm.
/// This is not sent through the wire.
#[derive(Debug, PartialEq)]
pub(crate) struct RequestRateLimitData {
    /// Maximum requests allowed by this request type.
    max_requests: u32,
    ///  The range/window of time of this request type.
    time_window: Duration,
}

impl RequestRateLimitData {
    pub(crate) fn new<Req: RequestCommon>() -> Self {
        Self {
            max_requests: Req::MAX_REQUESTS,
            time_window: Req::TIME_WINDOW,
        }
    }
}

/// Holds the expiration time for a given peer and request type. This struct defines the ordering for the btree set.
/// The smaller expiration times come first.
#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub(crate) struct Expiration {
    pub(crate) peer_id: PeerId,
    pub(crate) req_type: RequestType,
    pub(crate) expiration_time: Instant,
}

impl Expiration {
    pub(crate) fn new(peer_id: PeerId, req_type: RequestType, expiration_time: Instant) -> Self {
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
    by_peer_and_req_type: HashMap<(PeerId, RequestType), RateLimit>,
    /// The ordered set of rate limits by expiration time, peer id and request type.
    by_expiration_time: BTreeSet<Expiration>,
}

impl PendingDeletion {
    /// Retrieves the first item by expiration.
    pub(crate) fn first(&self) -> Option<&Expiration> {
        self.by_expiration_time.first()
    }

    /// Adds to both structures the new entry. If the entry already exists we replace it on both structs.
    pub(crate) fn insert(
        &mut self,
        peer_id: PeerId,
        req_type: RequestType,
        rate_limit: &RateLimit,
    ) {
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

// Network helpers for rate limiting
#[derive(Default)]
pub(crate) struct RateLimits {
    peer_request_limits: Mutex<HashMap<PeerId, HashMap<RequestType, RateLimit>>>,
    rate_limits_pending_deletion: Mutex<PendingDeletion>,
}

impl RateLimits {
    pub(crate) fn exceeds_rate_limit(
        &mut self,
        peer_id: PeerId,
        request_type: RequestType,
        request_id: InboundRequestId,
        request_rate_limit_data: &RequestRateLimitData,
    ) -> bool {
        // Gets lock of peer requests limits read and write on it.
        let mut peer_request_limits = self.peer_request_limits.lock();

        // If the peer has never sent a request of this type, creates a new entry.
        let requests_limit = peer_request_limits
            .entry(peer_id)
            .or_default()
            .entry(request_type)
            .or_insert_with(|| {
                RateLimit::new(
                    request_rate_limit_data.max_requests,
                    request_rate_limit_data.time_window,
                    Instant::now(),
                )
            });

        // Ensures that the request is allowed based on the set limits and updates the counter.
        // Returns early if not allowed.
        if !requests_limit.increment_and_is_allowed(1) {
            info!(
                %request_id,
                %peer_id,
                %request_type,
                "Rate limit was exceeded!",
            );
            log::debug!(
                "[{:?}][{:?}] {:?} Exceeded max requests rate {:?} requests per {:?} seconds",
                request_id,
                peer_id,
                request_type,
                request_rate_limit_data.max_requests,
                request_rate_limit_data.time_window,
            );
            return true;
        }

        false
    }

    pub(crate) fn remove_rate_limits(&mut self, peer_id: PeerId) {
        // Every time a peer disconnects, we delete all expired pending limits.
        self.clean_up();

        // Firstly we must acquire the lock of the pending deletes to avoid deadlocks.
        let mut rate_limits_pending_deletion_l = self.rate_limits_pending_deletion.lock();
        let mut peer_request_limits_l = self.peer_request_limits.lock();

        // Go through all existing request types of the given peer and deletes the limit counters if possible or marks it for deletion.
        if let Some(request_limits) = peer_request_limits_l.get_mut(&peer_id) {
            request_limits.retain(|req_type, rate_limit| {
                // Gets the requests limit and deletes it if no counter info would be lost, otherwise places it as pending deletion.
                if !rate_limit.can_delete(Instant::now()) {
                    rate_limits_pending_deletion_l.insert(peer_id, *req_type, rate_limit);
                    true
                } else {
                    false
                }
            });
            // If the peer no longer has any pending rate limits, then it gets removed.
            if request_limits.is_empty() {
                peer_request_limits_l.remove(&peer_id);
            }
        }
    }

    /// Deletes the rate limits that were previously marked as pending if its expiration time has passed.
    fn clean_up(&mut self) {
        let mut rate_limits_pending_deletion_l = self.rate_limits_pending_deletion.lock();

        // Iterates from the oldest to the most recent expiration date and deletes the entries that have expired.
        // The pending to deletion is ordered from the oldest to the most recent expiration date, thus we break early
        // from the loop once we find a non expired rate limit.
        while let Some(peer_expiration) = rate_limits_pending_deletion_l.first() {
            let current_timestamp = Instant::now();
            if peer_expiration.expiration_time <= current_timestamp {
                let mut peer_request_limits_l = self.peer_request_limits.lock();

                if let Some(peer_req_limits) = peer_request_limits_l
                    .get_mut(&peer_expiration.peer_id)
                    .and_then(|peer_req_limits| {
                        if let Some(rate_limit) = peer_req_limits.get(&peer_expiration.req_type) {
                            // If the peer has reconnected the rate limit may be enforcing a new limit. In this case we only remove
                            // the pending deletion.
                            if rate_limit.can_delete(current_timestamp) {
                                peer_req_limits.remove(&peer_expiration.req_type);
                            }
                            return Some(peer_req_limits);
                        }
                        // Only returns None if no request type was found.
                        None
                    })
                {
                    // If the peer no longer has any pending rate limits, then it gets removed from both rate limits and pending deletion.
                    if peer_req_limits.is_empty() {
                        peer_request_limits_l.remove(&peer_expiration.peer_id);
                    }
                } else {
                    // If the information is in pending deletion, that should mean it was not deleted from peer_request_limits yet, so that
                    // reconnection doesn't bypass the limits we are enforcing.
                    unreachable!(
                        "Tried to remove a non existing rate limit from peer_request_limits."
                    );
                }
                // Removes the entry from the pending for deletion.
                rate_limits_pending_deletion_l.remove_first();
            } else {
                break;
            }
        }
    }
}
