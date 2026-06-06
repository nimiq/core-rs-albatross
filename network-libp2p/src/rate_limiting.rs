use std::{
    cmp::Ordering,
    collections::{hash_map::Entry, BTreeSet, HashMap},
    time::Duration,
};

use instant::Instant;
use libp2p::{gossipsub::TopicHash, PeerId};
use nimiq_network_interface::{
    network::Topic,
    request::{RequestCommon, RequestType},
};

/// The rate limiting request metadata that will be passed on between the network and the swarm.
/// This is not sent through the wire.
#[derive(Debug, PartialEq)]
pub(crate) struct RateLimitConfig {
    /// Maximum requests allowed in the time window.
    pub(crate) max_requests: u32,
    ///  The range/window of time to consider.
    pub(crate) time_window: Duration,
}

impl RateLimitConfig {
    pub(crate) fn from_request<Req: RequestCommon>() -> Self {
        Self {
            max_requests: Req::MAX_REQUESTS,
            time_window: Req::TIME_WINDOW,
        }
    }

    pub(crate) fn from_topic<T: Topic>() -> Self {
        Self {
            max_requests: T::MAX_MESSAGES,
            time_window: T::TIME_WINDOW,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum RateLimitId {
    Request(RequestType),
    Gossipsub(TopicHash),
    /// Inbound DHT `PutRecord` requests. Rate limited per peer so a single peer
    /// cannot flood the swarm task with record verifications.
    #[cfg(feature = "kad")]
    DhtPutRecord,
}

/// Holds the expiration time for a given peer and request type. This struct defines the ordering for the btree set.
/// The smaller expiration times come first.
#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub(crate) struct Expiration {
    pub(crate) peer_id: PeerId,
    pub(crate) rate_limit_id: RateLimitId,
    pub(crate) expiration_time: Instant,
}

impl Expiration {
    pub(crate) fn new(
        peer_id: PeerId,
        rate_limit_id: RateLimitId,
        expiration_time: Instant,
    ) -> Self {
        Self {
            peer_id,
            rate_limit_id,
            expiration_time,
        }
    }
}

impl Ord for Expiration {
    fn cmp(&self, other: &Self) -> Ordering {
        self.expiration_time
            .cmp(&other.expiration_time)
            .then_with(|| self.peer_id.cmp(&other.peer_id))
            .then_with(|| self.rate_limit_id.cmp(&other.rate_limit_id))
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
    by_peer_and_id: HashMap<(PeerId, RateLimitId), RateLimit>,
    /// The ordered set of rate limits by expiration time, peer id and rate limit id.
    by_expiration_time: BTreeSet<Expiration>,
}

impl PendingDeletion {
    fn pop_first_if_expired(&mut self, current_time: Instant) -> Option<Expiration> {
        if self.by_expiration_time.first()?.expiration_time > current_time {
            return None;
        }
        let expiration = self
            .by_expiration_time
            .pop_first()
            .expect("First item must exist");
        assert!(
            self.by_peer_and_id
                .remove(&(expiration.peer_id, expiration.rate_limit_id.clone()))
                .is_some(),
            "The pending for deletion rate limits should be consistent among them"
        );
        Some(expiration)
    }

    /// Adds to both structures the new entry. If the entry already exists we replace it on both structs.
    pub(crate) fn insert(
        &mut self,
        peer_id: PeerId,
        rate_limit_id: RateLimitId,
        rate_limit: &RateLimit,
    ) {
        let key = (peer_id, rate_limit_id.clone());
        if let Some(expiration_peer) = self.by_peer_and_id.insert(key, rate_limit.clone()) {
            self.by_expiration_time.remove(&Expiration::new(
                peer_id,
                rate_limit_id.clone(),
                expiration_peer.next_reset_time(),
            ));
        }
        self.by_expiration_time.insert(Expiration::new(
            peer_id,
            rate_limit_id,
            rate_limit.next_reset_time(),
        ));
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

// Rate limiting overarching structure. It holds the rate limits by peer and request type.
// This handles the case of a peer reconnecting within the time window to attempt to bypass the rate limits established.
#[derive(Default)]
pub(crate) struct RateLimits {
    /// The rate limits per active peer.
    rate_limits: HashMap<PeerId, HashMap<RateLimitId, RateLimit>>,
    /// All the pending deletion rate limits.
    rate_limits_pending_deletion: PendingDeletion,
}

impl RateLimits {
    /// Increases the counter of the rate limit and returns a bool in case the defined rate limit is surpassed.
    pub(crate) fn exceeds_rate_limit(
        &mut self,
        peer_id: PeerId,
        rate_limit_id: RateLimitId,
        rate_limit_config: &RateLimitConfig,
    ) -> bool {
        // If the peer has never sent a request of this type, creates a new entry.
        let requests_limit = self
            .rate_limits
            .entry(peer_id)
            .or_default()
            .entry(rate_limit_id)
            .or_insert_with(|| {
                RateLimit::new(
                    rate_limit_config.max_requests,
                    rate_limit_config.time_window,
                    Instant::now(),
                )
            });

        // Ensures that the request is allowed based on the set limits and updates the counter.
        !requests_limit.increment_and_is_allowed(1)
    }

    /// Mark all rate limits of a given peer as pending for deletion.
    /// Every time this is called the expired rate limits will get delete pruned.
    pub(crate) fn remove_rate_limits(&mut self, peer_id: PeerId) {
        // Get a single timestamp as reference time.
        let current_time = Instant::now();
        // Every time a peer disconnects, we delete all expired pending limits.
        self.clean_up(current_time);

        // Go through all existing request types of the given peer and deletes the limit counters if possible or marks it for deletion.
        if let Some(rate_limits) = self.rate_limits.get_mut(&peer_id) {
            rate_limits.retain(|rate_limit_id, rate_limit| {
                // Gets the requests limit and deletes it if no counter info would be lost, otherwise places it as pending deletion.
                if !rate_limit.can_delete(current_time) {
                    self.rate_limits_pending_deletion.insert(
                        peer_id,
                        rate_limit_id.clone(),
                        rate_limit,
                    );
                    true
                } else {
                    false
                }
            });
            // If the peer no longer has any pending rate limits, then it gets removed.
            if rate_limits.is_empty() {
                self.rate_limits.remove(&peer_id);
            }
        }
    }

    /// Deletes the rate limits that were previously marked as pending if its expiration time has passed.
    fn clean_up(&mut self, current_time: Instant) {
        // Iterates from the oldest to the most recent expiration date and deletes the entries that have expired.
        // The pending to deletion is ordered from the oldest to the most recent expiration date, thus we break early
        // from the loop once we find a non expired rate limit.
        while let Some(expiration) = self
            .rate_limits_pending_deletion
            .pop_first_if_expired(current_time)
        {
            // For the tagged pending expiration, retrieve the rate limit of the related peer
            let Entry::Occupied(mut rate_limit_for_peer) =
                self.rate_limits.entry(expiration.peer_id)
            else {
                log::error!(
                    ?expiration,
                    "Tried to retrieve a non existing rate limit for a peer, which should exist"
                );
                // fail in debug builds, so the broken invariant surfaces
                debug_assert!(
                    false,
                    "Tried to retrieve a non existing rate limit for a peer, which should exist: {:?}",
                    expiration,
                );
                // Something broke the invariant here, but there is good reason to believe
                // this will self correct after the pop of the while loop above.
                continue;
            };
            //
            let Entry::Occupied(rate_limit) = rate_limit_for_peer
                .get_mut()
                .entry(expiration.rate_limit_id.clone())
            else {
                log::error!(
                    ?expiration,
                    "Tried to retrieve a non existing rate limit for an id, which should exist"
                );
                // fail in debug builds, so the broken invariant surfaces
                debug_assert!(
                    false,
                    "Tried to retrieve a non existing rate limit for an id, which should exist: {:?}",
                    expiration,
                );
                // Something broke the invariant here, but there is good reason to believe
                // this will self correct after the pop of the while loop above.
                continue;
            };

            if rate_limit.get().can_delete(current_time) {
                rate_limit.remove_entry();
            }
            if rate_limit_for_peer.get().is_empty() {
                rate_limit_for_peer.remove_entry();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::spin_loop;

    use super::*;

    fn rate_limit_with_expiration(expiration_time: Instant) -> RateLimit {
        let time_window = Duration::from_millis(10);
        let mut rate_limit = RateLimit::new(1, time_window, expiration_time - time_window);
        rate_limit.occurrences_counter = 1;
        rate_limit
    }

    fn rate_limits_with_pending_entry(
        peer_id: PeerId,
        rate_limit_id: RateLimitId,
        rate_limit: RateLimit,
        capacity: usize,
    ) -> RateLimits {
        let mut rate_limits = RateLimits::default();
        let mut peer_rate_limits = HashMap::with_capacity(capacity);
        peer_rate_limits.insert(rate_limit_id.clone(), rate_limit.clone());
        rate_limits.rate_limits.insert(peer_id, peer_rate_limits);
        rate_limits
            .rate_limits_pending_deletion
            .insert(peer_id, rate_limit_id, &rate_limit);
        rate_limits
    }

    fn wait_until(target: Instant) {
        while Instant::now() < target {
            spin_loop();
        }
    }

    #[test]
    fn remove_rate_limits_keeps_pending_deletions_consistent_at_expiry_boundary() {
        const INNER_CAPACITY: usize = 1 << 17;
        const START_OFFSETS: [Duration; 6] = [
            Duration::from_micros(2),
            Duration::from_micros(4),
            Duration::from_micros(8),
            Duration::from_micros(16),
            Duration::from_micros(32),
            Duration::from_micros(64),
        ];

        let peer_id = PeerId::random();
        let cleanup_peer_id = PeerId::random();
        let rate_limit_id = RateLimitId::Request(RequestType::request(42));
        let mut observed_pre_expiry_disconnect = false;

        for start_offset in START_OFFSETS {
            let expiration_time = Instant::now() + start_offset + Duration::from_millis(1);
            let rate_limit = rate_limit_with_expiration(expiration_time);
            let mut rate_limits = rate_limits_with_pending_entry(
                peer_id,
                rate_limit_id.clone(),
                rate_limit,
                INNER_CAPACITY,
            );

            wait_until(expiration_time - start_offset);
            rate_limits.remove_rate_limits(peer_id);

            let has_active_rate_limit = rate_limits
                .rate_limits
                .get(&peer_id)
                .and_then(|peer_rate_limits| peer_rate_limits.get(&rate_limit_id))
                .is_some();
            let has_pending_deletion = rate_limits
                .rate_limits_pending_deletion
                .by_peer_and_id
                .contains_key(&(peer_id, rate_limit_id.clone()));

            assert!(
                !has_pending_deletion || has_active_rate_limit,
                "disconnecting close to expiry must not leave a pending-deletion entry without the matching active rate limit",
            );

            if has_active_rate_limit {
                observed_pre_expiry_disconnect = true;

                wait_until(expiration_time + Duration::from_millis(1));
                rate_limits.remove_rate_limits(cleanup_peer_id);

                assert!(
                    !rate_limits.rate_limits.contains_key(&peer_id),
                    "the active rate limit should be removed once the pending deletion expires",
                );
                assert!(
                    !rate_limits
                        .rate_limits_pending_deletion
                        .by_peer_and_id
                        .contains_key(&(peer_id, rate_limit_id.clone())),
                    "the pending-deletion index should be cleared once the rate limit expires",
                );
                assert!(
                    !rate_limits
                        .rate_limits_pending_deletion
                        .by_expiration_time
                        .iter()
                        .any(|expiration| {
                            expiration.peer_id == peer_id
                                && expiration.rate_limit_id == rate_limit_id
                        }),
                    "the expiration index should be cleared once the rate limit expires",
                );
            }
        }

        assert!(
            observed_pre_expiry_disconnect,
            "the test must exercise at least one disconnect that happens before the rate limit expires",
        );
    }

    #[cfg(feature = "kad")]
    #[test]
    fn dht_put_rate_limit_is_enforced_per_peer() {
        let config = RateLimitConfig {
            max_requests: 3,
            time_window: Duration::from_secs(60),
        };
        let peer_id = PeerId::random();
        let mut rate_limits = RateLimits::default();

        // The first `max_requests` puts from a peer are allowed.
        for _ in 0..config.max_requests {
            assert!(
                !rate_limits.exceeds_rate_limit(peer_id, RateLimitId::DhtPutRecord, &config),
                "puts within the limit must be allowed",
            );
        }

        // Any further put within the same window is dropped.
        assert!(
            rate_limits.exceeds_rate_limit(peer_id, RateLimitId::DhtPutRecord, &config),
            "puts exceeding the limit must be dropped",
        );

        // The limit is tracked per peer, so a different peer is unaffected.
        assert!(
            !rate_limits.exceeds_rate_limit(PeerId::random(), RateLimitId::DhtPutRecord, &config),
            "a different peer must have its own independent rate limit",
        );
    }
}
