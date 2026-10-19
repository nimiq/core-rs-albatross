use std::{
    collections::{BTreeSet, HashMap},
    hash::Hash,
    num::NonZeroU32,
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
    /// Maximum number of requests a peer can make in a burst. It is also the number of requests
    /// that get refilled over `time_window`.
    max_requests: NonZeroU32,
    /// The time it takes to refill the whole budget of `max_requests`. At least a nanosecond per
    /// request.
    time_window: Duration,
}

impl RateLimitConfig {
    /// Panics on a budget of zero requests and on a time window of less than a nanosecond per
    /// request, which includes a zero window. The first would refuse every request and the second
    /// would floor to a refill interval of zero. Outside of the tests every config is built at
    /// compile time, where the panic is a build error.
    pub(crate) const fn new(max_requests: u32, time_window: Duration) -> Self {
        let max_requests = NonZeroU32::new(max_requests)
            .expect("a rate limit needs a budget of at least one request");
        assert!(
            time_window.as_nanos() >= max_requests.get() as u128,
            "a rate limit needs a time window of at least a nanosecond per request",
        );
        Self {
            max_requests,
            time_window,
        }
    }

    /// The rate limit of a request type. A request type with an invalid one does not build.
    pub(crate) fn from_request<Req: RequestCommon>() -> Self {
        const { Self::new(Req::MAX_REQUESTS, Req::TIME_WINDOW) }
    }

    /// The rate limit of a gossipsub topic. A topic with an invalid one does not build.
    pub(crate) fn from_topic<T: Topic>() -> Self {
        const { Self::new(T::MAX_MESSAGES, T::TIME_WINDOW) }
    }

    pub(crate) fn max_requests(&self) -> NonZeroU32 {
        self.max_requests
    }

    pub(crate) fn time_window(&self) -> Duration {
        self.time_window
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) enum RateLimitId {
    Request(RequestType),
    Gossipsub(TopicHash),
    /// Inbound DHT `PutRecord` requests. Rate limited per peer so a single peer
    /// cannot flood the swarm task with record verifications.
    #[cfg(feature = "kad")]
    DhtPutRecord,
}

/// Token buckets for a single rate limit, one bucket per id.
///
/// Every id starts out with a full bucket of `capacity` tokens. Each request takes one token and
/// a request is refused when the bucket is empty, without changing anything. Tokens come back one
/// every `refill_interval`, so a peer that bursts regains capacity gradually instead of waiting
/// for a whole window to pass. A bucket that is full again is dropped: a missing bucket is a full
/// one. This is also why a peer's budget needs no bookkeeping when it disconnects, it stays spent
/// until it has refilled, whether the peer reconnects or not. Full buckets are dropped whenever
/// the refills are applied, which every take does for its own buckets and [`RateLimits::sweep`]
/// does for all of them.
#[derive(Debug)]
pub(crate) struct TokenBuckets<Id> {
    /// Maximum number of tokens in a bucket.
    capacity: NonZeroU32,
    /// Time it takes to add one token to a bucket. Never zero, see [`RateLimitConfig::new`].
    refill_interval: Duration,
    /// The buckets that are not full, by id.
    buckets: HashMap<Id, u32>,
    /// When each non-full bucket gets its next token, earliest first.
    refills: BTreeSet<(Instant, Id)>,
}

impl<Id: Clone + Eq + Hash + Ord> TokenBuckets<Id> {
    pub(crate) fn new(config: &RateLimitConfig) -> Self {
        Self {
            capacity: config.max_requests,
            refill_interval: Self::refill_interval(config),
            buckets: HashMap::new(),
            refills: BTreeSet::new(),
        }
    }

    /// The time it takes to add one token to a bucket of `config`.
    fn refill_interval(config: &RateLimitConfig) -> Duration {
        config.time_window / config.max_requests.get()
    }

    /// Whether these buckets were created from `config`.
    fn is_from(&self, config: &RateLimitConfig) -> bool {
        self.capacity == config.max_requests
            && self.refill_interval == Self::refill_interval(config)
    }

    /// Tries to take one token for `id` at time `now`. Returns whether the token was taken.
    /// A refusal takes nothing, only the refills that became due are applied.
    pub(crate) fn try_take(&mut self, id: Id, now: Instant) -> bool {
        self.refill(now);

        match self.buckets.get_mut(&id) {
            Some(tokens) => match tokens.checked_sub(1) {
                Some(remaining) => {
                    *tokens = remaining;
                    true
                }
                None => false,
            },
            None => {
                // A missing bucket is a full one, and the capacity is at least one token.
                self.buckets.insert(id.clone(), self.capacity.get() - 1);
                self.refills.insert((now + self.refill_interval, id));
                true
            }
        }
    }

    /// Adds the tokens that became due since the last call and drops the buckets that are full
    /// again. The refill interval is never zero, so the division below never sees a zero.
    fn refill(&mut self, now: Instant) {
        while let Some((due, _)) = self.refills.first() {
            if *due > now {
                break;
            }
            let (due, id) = self
                .refills
                .pop_first()
                .expect("checked by the loop condition");
            let Some(tokens) = self.buckets.get_mut(&id) else {
                debug_assert!(false, "every scheduled refill has a bucket");
                continue;
            };

            // The token that is due now plus every one that became due since, capped at what the
            // bucket can still hold. Only non-full buckets are kept, so `missing` is at least one;
            // if a full one got in anyway, `added == missing == 0` drops it below.
            let missing = self.capacity.get().saturating_sub(*tokens);
            let due_tokens = 1 + (now - due).as_nanos() / self.refill_interval.as_nanos();
            let added = u32::try_from(due_tokens).unwrap_or(u32::MAX).min(missing);

            if added == missing {
                self.buckets.remove(&id);
            } else {
                *tokens += added;
                self.refills
                    .insert((due + self.refill_interval * added, id));
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buckets.len()
    }
}

/// The rate limits of all peers, one set of token buckets per rate limit id.
#[derive(Default)]
pub(crate) struct RateLimits {
    limits: HashMap<RateLimitId, TokenBuckets<PeerId>>,
}

impl RateLimits {
    /// Takes one token from the bucket of `peer_id` for `rate_limit_id` and returns whether the
    /// rate limit was exceeded instead, in which case nothing is taken. The config is used the
    /// first time a rate limit id is seen and must be the same on every later call, which debug
    /// builds check.
    pub(crate) fn exceeds_rate_limit(
        &mut self,
        peer_id: PeerId,
        rate_limit_id: RateLimitId,
        rate_limit_config: &RateLimitConfig,
        now: Instant,
    ) -> bool {
        let buckets = self
            .limits
            .entry(rate_limit_id)
            .or_insert_with(|| TokenBuckets::new(rate_limit_config));
        debug_assert!(
            buckets.is_from(rate_limit_config),
            "a rate limit id must always come with the same config",
        );
        !buckets.try_take(peer_id, now)
    }

    /// Drops the buckets that are full again at time `now`, for every rate limit. A take does
    /// this for its own rate limit, so this only matters for rate limits that get no requests
    /// for a while, typically because the peers that used them left. Nothing else changes: a
    /// budget that is not full yet stays as spent as it is.
    pub(crate) fn sweep(&mut self, now: Instant) {
        for buckets in self.limits.values_mut() {
            buckets.refill(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn buckets(max_requests: u32, time_window: Duration) -> TokenBuckets<u32> {
        TokenBuckets::new(&RateLimitConfig::new(max_requests, time_window))
    }

    #[test]
    fn allows_a_burst_of_the_whole_budget_and_then_refuses() {
        let mut buckets = buckets(3, 3 * SECOND);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(buckets.try_take(1, now));
        }
        assert!(!buckets.try_take(1, now));
        // A refusal does not change the bucket.
        assert!(!buckets.try_take(1, now + SECOND / 2));
        assert_eq!(buckets.len(), 1);
    }

    #[test]
    fn refills_one_token_per_interval() {
        let mut buckets = buckets(3, 3 * SECOND);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(buckets.try_take(1, now));
        }
        // Nothing before the interval has passed, exactly one token once it has.
        assert!(!buckets.try_take(1, now + SECOND - Duration::from_nanos(1)));
        assert!(buckets.try_take(1, now + SECOND));
        assert!(!buckets.try_take(1, now + SECOND));
        // Two more intervals, two more tokens.
        let later = now + 3 * SECOND;
        assert!(buckets.try_take(1, later));
        assert!(buckets.try_take(1, later));
        assert!(!buckets.try_take(1, later));
    }

    #[test]
    fn refills_do_not_drift_when_they_are_observed_late() {
        let mut buckets = buckets(3, 3 * SECOND);
        let start = Instant::now();
        for _ in 0..3 {
            assert!(buckets.try_take(1, start));
        }
        // The first refill is only observed at 2.5s: the tokens due at 1s and 2s are both there.
        let late = start + Duration::from_millis(2500);
        assert!(buckets.try_take(1, late));
        assert!(buckets.try_take(1, late));
        assert!(!buckets.try_take(1, late));
        // The third token is still due at 3s, not one interval after the late refill.
        assert!(!buckets.try_take(1, start + Duration::from_millis(2999)));
        assert!(buckets.try_take(1, start + Duration::from_millis(3000)));
    }

    #[test]
    fn a_full_bucket_does_not_accumulate() {
        // One token per second with a capacity of one: a taker that comes every 1.2s gets a token
        // every time, a taker that comes every 0.6s only every other time.
        let mut buckets = buckets(1, SECOND);
        let start = Instant::now();
        assert!(buckets.try_take(1, start));
        let mut granted = Vec::new();
        for step in 1..=10u64 {
            let now = start + Duration::from_millis(600 * step);
            if buckets.try_take(1, now) {
                granted.push(600 * step);
            }
        }
        assert_eq!(granted, vec![1200, 2400, 3600, 4800, 6000]);
    }

    #[test]
    fn drops_buckets_that_are_full_again() {
        let mut buckets = buckets(2, 2 * SECOND);
        let now = Instant::now();
        assert!(buckets.try_take(1, now));
        assert!(buckets.try_take(2, now));
        assert!(buckets.try_take(2, now));
        assert_eq!(buckets.len(), 2);
        // Peer 1 is full again after one interval, peer 2 after two. Any take runs the cleanup.
        assert!(buckets.try_take(3, now + SECOND));
        assert_eq!(buckets.len(), 2);
        assert!(!buckets.buckets.contains_key(&1));
        assert!(buckets.try_take(3, now + 2 * SECOND));
        assert_eq!(buckets.len(), 1);
        assert!(buckets.buckets.contains_key(&3));
    }

    #[test]
    fn a_full_or_overfull_bucket_is_dropped_on_refill() {
        // `try_take` never leaves a full bucket behind. If one gets in anyway, the next refill
        // drops it instead of underflowing `missing`.
        let mut buckets = buckets(2, 2 * SECOND);
        let now = Instant::now();
        buckets.buckets.insert(1, 2);
        buckets.buckets.insert(2, 5);
        buckets.refills.insert((now, 1));
        buckets.refills.insert((now, 2));
        assert!(buckets.try_take(3, now));
        assert_eq!(buckets.len(), 1);
        assert!(buckets.buckets.contains_key(&3));
        assert_eq!(buckets.refills.len(), 1);
    }

    #[test]
    fn ids_are_independent() {
        let mut buckets = buckets(1, SECOND);
        let now = Instant::now();
        assert!(buckets.try_take(1, now));
        assert!(!buckets.try_take(1, now));
        assert!(buckets.try_take(2, now));
    }

    #[test]
    #[should_panic(expected = "at least one request")]
    fn a_zero_budget_is_a_misconfiguration() {
        RateLimitConfig::new(0, SECOND);
    }

    #[test]
    #[should_panic(expected = "at least a nanosecond per request")]
    fn a_zero_window_is_a_misconfiguration() {
        RateLimitConfig::new(1, Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "at least a nanosecond per request")]
    fn a_sub_nanosecond_interval_is_a_misconfiguration() {
        RateLimitConfig::new(2, Duration::from_nanos(1));
    }

    #[test]
    fn buckets_are_full_and_dropped_after_capacity_intervals_of_idle_time() {
        for _ in 0..200 {
            let capacity = rand::random::<u32>() % 10 + 1;
            let interval = Duration::from_millis(rand::random::<u64>() % 1000 + 1);
            let mut buckets = buckets(capacity, interval * capacity);
            let mut now = Instant::now();
            for _ in 0..rand::random::<u32>() % 100 {
                now += interval.mul_f64(rand::random::<f64>() * 2.0);
                buckets.try_take(rand::random::<u32>() % 5, now);
                // Every non-full bucket has exactly one scheduled refill.
                assert_eq!(buckets.refills.len(), buckets.buckets.len());
            }
            // After `capacity` intervals of idle time every bucket is full and gone.
            now += interval * capacity;
            assert!(buckets.try_take(0, now));
            assert_eq!(buckets.len(), 1);
            assert_eq!(buckets.refills.len(), 1);
        }
    }

    #[test]
    fn a_huge_idle_gap_is_capped_at_the_capacity() {
        // One token per nanosecond: after five seconds more tokens are due than fit in a `u32`.
        // The count saturates and is capped at what the bucket can hold, so the bucket is simply
        // full again and dropped.
        let mut buckets = buckets(2, Duration::from_nanos(2));
        let now = Instant::now();
        assert!(buckets.try_take(1, now));
        assert!(buckets.try_take(1, now));
        assert!(!buckets.try_take(1, now));
        let later = now + 5 * SECOND;
        assert!(buckets.try_take(1, later));
        assert!(buckets.try_take(1, later));
        assert!(!buckets.try_take(1, later));
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets.refills.len(), 1);
    }

    #[test]
    fn rate_limits_keep_one_bucket_set_per_id() {
        let mut limits = RateLimits::default();
        let peer_id = PeerId::random();
        let now = Instant::now();
        let strict = RateLimitConfig::new(1, SECOND);
        let lax = RateLimitConfig::new(10, SECOND);
        let strict_id = RateLimitId::Request(RequestType::request(1));
        let lax_id = RateLimitId::Request(RequestType::request(2));

        assert!(!limits.exceeds_rate_limit(peer_id, strict_id.clone(), &strict, now));
        assert!(limits.exceeds_rate_limit(peer_id, strict_id.clone(), &strict, now));
        for _ in 0..10 {
            assert!(!limits.exceeds_rate_limit(peer_id, lax_id.clone(), &lax, now));
        }
        assert!(limits.exceeds_rate_limit(peer_id, lax_id, &lax, now));
        // Another peer has its own bucket.
        assert!(!limits.exceeds_rate_limit(PeerId::random(), strict_id, &strict, now));
    }

    #[test]
    fn a_sweep_drops_only_the_buckets_that_are_full_again() {
        fn non_full_buckets(limits: &RateLimits) -> usize {
            limits.limits.values().map(TokenBuckets::len).sum()
        }

        let mut limits = RateLimits::default();
        let now = Instant::now();
        let config = RateLimitConfig::new(2, 2 * SECOND);
        let (peer_1, peer_2) = (PeerId::random(), PeerId::random());
        let id_1 = RateLimitId::Request(RequestType::request(1));
        let id_2 = RateLimitId::Request(RequestType::request(2));

        // Peer 1 spends its whole budget on both ids, peer 2 one token on the first.
        for _ in 0..2 {
            assert!(!limits.exceeds_rate_limit(peer_1, id_1.clone(), &config, now));
            assert!(!limits.exceeds_rate_limit(peer_1, id_2.clone(), &config, now));
        }
        assert!(!limits.exceeds_rate_limit(peer_2, id_1.clone(), &config, now));
        assert_eq!(non_full_buckets(&limits), 3);

        // One interval later peer 2 is full again and gone, peer 1 has one token back per id.
        limits.sweep(now + SECOND);
        assert_eq!(non_full_buckets(&limits), 2);
        assert!(!limits.exceeds_rate_limit(peer_1, id_1.clone(), &config, now + SECOND));
        assert!(limits.exceeds_rate_limit(peer_1, id_1, &config, now + SECOND));
        assert!(!limits.exceeds_rate_limit(peer_1, id_2.clone(), &config, now + SECOND));
        assert!(limits.exceeds_rate_limit(peer_1, id_2, &config, now + SECOND));

        // A whole window later everything is full and gone without any take. The bucket sets
        // themselves stay.
        limits.sweep(now + 3 * SECOND);
        assert_eq!(non_full_buckets(&limits), 0);
        assert_eq!(limits.limits.len(), 2);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "same config")]
    fn a_rate_limit_id_must_always_come_with_the_same_config() {
        let mut limits = RateLimits::default();
        let peer_id = PeerId::random();
        let now = Instant::now();
        let id = RateLimitId::Request(RequestType::request(1));
        let strict = RateLimitConfig::new(1, SECOND);
        let lax = RateLimitConfig::new(10, SECOND);
        assert!(!limits.exceeds_rate_limit(peer_id, id.clone(), &strict, now));
        limits.exceeds_rate_limit(peer_id, id, &lax, now);
    }

    #[cfg(feature = "kad")]
    #[test]
    fn dht_put_rate_limit_is_enforced_per_peer() {
        let config = RateLimitConfig::new(3, Duration::from_secs(60));
        let peer_id = PeerId::random();
        let now = Instant::now();
        let mut rate_limits = RateLimits::default();

        // The first `max_requests` puts from a peer are allowed.
        for _ in 0..config.max_requests().get() {
            assert!(
                !rate_limits.exceeds_rate_limit(peer_id, RateLimitId::DhtPutRecord, &config, now),
                "puts within the limit must be allowed",
            );
        }

        // Any further put is dropped until a token has refilled.
        assert!(
            rate_limits.exceeds_rate_limit(peer_id, RateLimitId::DhtPutRecord, &config, now),
            "puts exceeding the limit must be dropped",
        );

        // The limit is tracked per peer, so a different peer is unaffected.
        assert!(
            !rate_limits.exceeds_rate_limit(
                PeerId::random(),
                RateLimitId::DhtPutRecord,
                &config,
                now
            ),
            "a different peer must have its own independent rate limit",
        );
    }
}
