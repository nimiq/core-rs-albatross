use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};

use libp2p::PeerId;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::{
    network::Network as NetworkInterface, validator_claim::ValidatorClaim,
};
use nimiq_utils::tagged_signing::TaggedSigned;
use parking_lot::Mutex;

use crate::Network;

/// A [`ValidatorClaim`] together with its signature, as reconstructed from a peer contact.
pub type SignedValidatorClaim =
    TaggedSigned<ValidatorClaim<<Network as NetworkInterface>::PeerId>, KeyPair>;

/// Why a validator claim could not be checked at this point in time.
///
/// These outcomes are transient: the same claim may verify later, once this node has the state
/// required to check it. Contacts carrying such a claim are kept and re-checked periodically.
///
/// Every current reason is *node-wide*: it depends only on this node's own state (whether it has
/// a verifier, runs a light blockchain, or has a complete staking contract), never on the claim
/// being checked. The verifiers return them before even looking at the claimed validator address.
/// So once one claim comes back with such a reason, every other claim checked right now would
/// too, and checking them only costs blockchain reads. See [`UnverifiableReason::is_node_wide`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnverifiableReason {
    /// This node has no verifier wired up (e.g. the web client).
    NoVerifier,
    /// This node runs a light blockchain and cannot read the staking contract.
    LightClient,
    /// The staking contract is not (yet) complete on this node.
    StateIncomplete,
}

impl UnverifiableReason {
    /// Whether this reason depends only on this node's own state, never on the claim that was
    /// checked, so that every other claim checked before this node's state changes would come back
    /// unverifiable as well.
    ///
    /// Callers use this to stop spending blockchain reads on further checks until the next
    /// house-keeping tick (see [`ValidatorClaimBudget::exhaust`]). The match is deliberately
    /// exhaustive: a future reason that depends on the claim itself (for example, a lookup that
    /// failed for this one validator only) must return `false` here, or a single such claim would
    /// stall verification of every other claim for the rest of the tick.
    pub fn is_node_wide(&self) -> bool {
        match self {
            UnverifiableReason::NoVerifier
            | UnverifiableReason::LightClient
            | UnverifiableReason::StateIncomplete => true,
        }
    }
}

/// Why a validator claim is considered bogus.
///
/// These outcomes are conclusive for the claim as presented, but they are *not* grounds for
/// dropping the connection: an honest validator can present a claim we reject, for example while
/// its registration transaction is still pending or right after a signing key rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidReason {
    /// The staking contract has no validator with the claimed address.
    UnknownValidator,
    /// The signature does not verify against the validator's on-chain signing key.
    InvalidSignature,
}

/// The outcome of checking the validator claim carried by a peer contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidatorVerification {
    /// The claim was checked against the staking contract and holds.
    Verified,
    /// The claim could not be checked yet. Re-check later.
    Unverifiable(UnverifiableReason),
    /// The claim was checked and does not hold.
    Invalid(InvalidReason),
}

impl ValidatorVerification {
    /// Whether the claim could not be checked for a reason that applies to every claim alike.
    /// See [`UnverifiableReason::is_node_wide`].
    pub fn is_node_wide_unverifiable(&self) -> bool {
        matches!(self, ValidatorVerification::Unverifiable(reason) if reason.is_node_wide())
    }
}

/// Checks the validator claims carried by peer contacts against the staking contract.
///
/// This deliberately verifies *only* the binding of validator address to signing key and the
/// signature itself. Checks that are specific to DHT records (publisher identity, record key,
/// timestamp drift) stay in the DHT verifier, because a peer contact carries a different
/// timestamp unit and has no publisher.
pub trait ValidatorClaimVerifier: Send + Sync {
    fn verify_validator_claim(&self, signed_claim: &SignedValidatorClaim) -> ValidatorVerification;
}

/// A verifier for nodes that cannot check validator claims at all.
///
/// Every claim comes back as [`UnverifiableReason::NoVerifier`], so no contact is ever indexed as
/// belonging to a validator. A contact carrying a claim is still stored and can be dialed, but it
/// is never passed on to other peers (see
/// [`PeerContactInfo::is_gossipable`](super::peer_contacts::PeerContactInfo::is_gossipable)), so
/// a node using this verifier does not relay the contact of any validator that attaches a claim.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopValidatorClaimVerifier;

impl ValidatorClaimVerifier for NoopValidatorClaimVerifier {
    fn verify_validator_claim(
        &self,
        _signed_claim: &SignedValidatorClaim,
    ) -> ValidatorVerification {
        ValidatorVerification::Unverifiable(UnverifiableReason::NoVerifier)
    }
}

/// Bounds how many validator claims carried by peer contacts this node will verify against the
/// staking contract per house-keeping tick, across *all* discovery connections combined.
///
/// Verifying a claim takes the blockchain read lock, and an unauthenticated peer can hand us one
/// in its very first handshake message. Without a shared, aggregate cap, an attacker opening many
/// connections (each carrying up to [`Config::update_limit`](super::behaviour::Config) claims)
/// could force an unbounded burst of blockchain-state reads. A claim that doesn't fit in the
/// current budget is simply left unverified; the periodic re-check sweep
/// (`PeerContactBook::unverified_validator_contacts`) checks it later.
///
/// The budget has two counters, both refilled by [`Self::reset`]:
///
///  - The *general* budget, taken by [`Self::try_consume`], which any claim can spend.
///  - The *refresh reserve*, taken first by [`Self::try_consume_refresh`], for refreshed contacts
///    of peers that already hold a live verified binding to the address they claim (see
///    `PeerContactBook::has_live_verified_binding`). Without it, anyone could exhaust the general
///    budget with a couple of connections full of bogus claims under fresh peer keys, leaving an
///    honest validator's refreshed contact pending and therefore not gossiped. Once the reserve is
///    empty, refreshes fall back to the general budget.
///
/// Each validator address takes at most one unit of the reserve per tick; any further refresh
/// claiming it falls back to the general budget. A binding can only be obtained with the
/// validator's signing key, but anybody can relay a validator's genuine refreshed contact to us,
/// and every copy that arrives before the first one is stored is new to the contact book: several
/// copies in one handshake or peer-address update, or on several connections at once. Without the
/// per-validator limit, an unauthenticated peer could drain the reserve with a few batches of
/// copies of a few validators' refreshed contacts. With it, relaying such a copy at most gets that
/// one refresh verified, which is what the reserve is for, and draining the reserve takes as many
/// distinct validators as it has units, however many peer IDs each of them binds.
///
/// [`Self::exhaust`] empties both counters until the next reset, for when a check shows that no
/// claim can be verified by this node right now (see [`UnverifiableReason::is_node_wide`]).
///
/// Independently of both counters, only a few peers claiming the same validator address can
/// verify a contact per tick, one contact each (see [`Self::may_verify`]). A validator can sign
/// claims for as many peer IDs as it likes, and refresh each of them every second. Without this
/// limit a single one could spend the general budget of every node its contacts reach, since
/// verified contacts are gossiped, keeping other validators' first claims from being verified.
/// Only claims that verify count towards the limit, so bogus claims naming an honest validator's
/// address cannot use up its share. Counting peers rather than contacts means that replaying a
/// peer's genuine older contacts, one after the other, takes up a single place in the share.
#[derive(Debug)]
pub struct ValidatorClaimBudget {
    /// What [`Self::reset`] refills the general budget to.
    capacity: usize,
    /// What [`Self::reset`] refills the refresh reserve to.
    refresh_reserve_capacity: usize,
    /// The general budget left until the next reset.
    remaining: AtomicUsize,
    /// The refresh reserve left until the next reset.
    remaining_refresh_reserve: AtomicUsize,
    /// The validator addresses that took a unit of the refresh reserve since the last reset.
    ///
    /// Every entry costs a unit of the reserve, so this never holds more than
    /// `refresh_reserve_capacity` addresses. Taking a unit happens under this lock, so concurrent
    /// refreshes of the same validator cannot both take one. The lock is never held while a
    /// claim is checked, nor while any other lock is taken.
    refreshed_validators: Mutex<HashSet<Address>>,
    /// How many distinct peers claiming the same validator address can verify a contact per tick.
    verified_claims_per_validator: usize,
    /// For each validator address, the peers whose claim to it verified since the last reset,
    /// each with the timestamp of the contact that verified.
    ///
    /// Entries are only added for claims that verified, i.e. that the validator signed, so this
    /// only holds addresses of validators that signed claims this tick. The limit is checked
    /// before a claim is checked and entries are added after, so claims checked at the same time
    /// on several discovery connections can exceed it by up to that many. That only costs a few
    /// more checks: bindings are capped per validator by the contact book anyway. The lock is
    /// never held while a claim is checked, nor while any other lock is taken.
    verified_claims: Mutex<HashMap<Address, HashMap<PeerId, u64>>>,
}

impl ValidatorClaimBudget {
    /// A budget of `capacity` claims per tick, without a refresh reserve.
    #[cfg(test)]
    pub(crate) fn new(capacity: usize) -> Self {
        Self::with_refresh_reserve(capacity, 0)
    }

    /// A budget of `capacity` claims per tick that any claim can spend, plus a reserve of
    /// `refresh_reserve` claims per tick that only [`Self::try_consume_refresh`] can spend.
    ///
    /// It does not limit how many peers claiming the same validator can verify per tick; see
    /// [`Self::with_per_validator_limit`].
    pub fn with_refresh_reserve(capacity: usize, refresh_reserve: usize) -> Self {
        Self {
            capacity,
            refresh_reserve_capacity: refresh_reserve,
            remaining: AtomicUsize::new(capacity),
            remaining_refresh_reserve: AtomicUsize::new(refresh_reserve),
            refreshed_validators: Mutex::new(HashSet::new()),
            verified_claims_per_validator: usize::MAX,
            verified_claims: Mutex::new(HashMap::new()),
        }
    }

    /// Limits how many distinct peers claiming the same validator address can verify a contact per
    /// tick to `verified_claims_per_validator`. See [`Self::may_verify`].
    pub fn with_per_validator_limit(mut self, verified_claims_per_validator: usize) -> Self {
        self.verified_claims_per_validator = verified_claims_per_validator;
        self
    }

    /// Whether the claim of the contact of `peer_id` timestamped `timestamp` to
    /// `validator_address` is worth checking this tick: either this very contact already verified
    /// this tick (e.g. this is another copy of it), or no contact of `peer_id` did and fewer peers
    /// than the limit claiming that validator verified one.
    ///
    /// A claim that is not worth checking should be left pending, without spending any budget on
    /// it. It is checked on a later tick, or superseded by a newer contact.
    pub fn may_verify(
        &self,
        validator_address: &Address,
        peer_id: &PeerId,
        timestamp: u64,
    ) -> bool {
        let verified_claims = self.verified_claims.lock();
        let peers = verified_claims.get(validator_address);
        match peers.and_then(|peers| peers.get(peer_id)) {
            Some(verified_timestamp) => *verified_timestamp == timestamp,
            None => peers.map_or(0, HashMap::len) < self.verified_claims_per_validator,
        }
    }

    /// Records that the claim of the contact of `peer_id` timestamped `timestamp` to
    /// `validator_address` verified. See [`Self::may_verify`].
    pub fn record_verified(&self, validator_address: &Address, peer_id: PeerId, timestamp: u64) {
        self.verified_claims
            .lock()
            .entry(validator_address.clone())
            .or_default()
            .insert(peer_id, timestamp);
    }

    /// Tries to consume one unit of the general budget. Returns whether one was available.
    ///
    /// This never touches the refresh reserve.
    pub fn try_consume(&self) -> bool {
        Self::try_take(&self.remaining)
    }

    /// Tries to consume one unit for a refresh from a peer that already holds a live verified
    /// binding to `validator_address`, the address its new contact claims: from the refresh
    /// reserve if `validator_address` has not taken a unit of it yet this tick and any is left,
    /// otherwise from the general budget. Returns whether one was available.
    ///
    /// The caller must have established that the binding exists; anything else has to use
    /// [`Self::try_consume`].
    pub fn try_consume_refresh(&self, validator_address: &Address) -> bool {
        {
            let mut refreshed_validators = self.refreshed_validators.lock();
            if !refreshed_validators.contains(validator_address)
                && Self::try_take(&self.remaining_refresh_reserve)
            {
                refreshed_validators.insert(validator_address.clone());
                return true;
            }
        }
        self.try_consume()
    }

    /// Empties the general budget and the refresh reserve until the next [`Self::reset`].
    ///
    /// This applies to whatever tick is current when it is called. A check charged to one tick
    /// that only returns after the next tick's reset can thus empty that next tick's budget too.
    /// That takes this node's state to change during that very check (e.g. its staking contract
    /// to complete), so it happens at most once per such change, and nobody else can trigger it.
    /// The claims left pending for the rest of that tick are still re-checked by the sweep, which
    /// this budget does not limit, one tick later than they would have been.
    pub fn exhaust(&self) {
        self.remaining.store(0, Ordering::Relaxed);
        self.remaining_refresh_reserve.store(0, Ordering::Relaxed);
    }

    /// Refills the general budget and the refresh reserve for the next tick, lets every
    /// validator take a unit of the reserve again, and restores every validator's share of
    /// verified claims.
    pub fn reset(&self) {
        {
            let mut refreshed_validators = self.refreshed_validators.lock();
            refreshed_validators.clear();
            self.remaining_refresh_reserve
                .store(self.refresh_reserve_capacity, Ordering::Relaxed);
        }
        self.verified_claims.lock().clear();
        self.remaining.store(self.capacity, Ordering::Relaxed);
    }

    /// The general budget and the refresh reserve left until the next reset.
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> (usize, usize) {
        (
            self.remaining.load(Ordering::Relaxed),
            self.remaining_refresh_reserve.load(Ordering::Relaxed),
        )
    }

    /// Takes one unit from `counter`, unless it is empty. Returns whether one was available.
    fn try_take(counter: &AtomicUsize) -> bool {
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| r.checked_sub(1))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;

    use super::*;

    /// A validator address for the budget to tell refreshes apart by.
    fn validator(seed: u8) -> Address {
        Address::from([seed; 20])
    }

    #[test]
    fn every_current_unverifiable_reason_is_node_wide() {
        for reason in [
            UnverifiableReason::NoVerifier,
            UnverifiableReason::LightClient,
            UnverifiableReason::StateIncomplete,
        ] {
            assert!(reason.is_node_wide());
            assert!(ValidatorVerification::Unverifiable(reason).is_node_wide_unverifiable());
        }
        assert!(!ValidatorVerification::Verified.is_node_wide_unverifiable());
        assert!(
            !ValidatorVerification::Invalid(InvalidReason::InvalidSignature)
                .is_node_wide_unverifiable()
        );
    }

    #[test]
    fn exhaust_empties_both_counters_until_reset_refills_them() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(3, 2);
        assert_eq!(budget.remaining(), (3, 2));

        budget.exhaust();
        assert_eq!(budget.remaining(), (0, 0));
        assert!(!budget.try_consume());
        assert!(!budget.try_consume_refresh(&validator(1)));

        budget.reset();
        assert_eq!(budget.remaining(), (3, 2));

        // A reset also refills counters that were only partially used.
        assert!(budget.try_consume());
        assert!(budget.try_consume_refresh(&validator(1)));
        budget.reset();
        assert_eq!(budget.remaining(), (3, 2));
    }

    #[test]
    fn a_refresh_takes_from_the_reserve_first_then_from_the_general_budget() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 2);

        assert!(budget.try_consume_refresh(&validator(1)));
        assert!(budget.try_consume_refresh(&validator(2)));
        assert_eq!(budget.remaining(), (1, 0));

        // With the reserve used up, a refresh falls back to the general budget...
        assert!(budget.try_consume_refresh(&validator(3)));
        assert_eq!(budget.remaining(), (0, 0));

        // ...and once that is gone too, it is refused.
        assert!(!budget.try_consume_refresh(&validator(4)));
        assert_eq!(budget.remaining(), (0, 0));
    }

    #[test]
    fn a_validator_takes_at_most_one_unit_of_the_reserve_per_tick() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 3);

        assert!(budget.try_consume_refresh(&validator(1)));
        assert_eq!(budget.remaining(), (1, 2));

        // Another refresh claiming the same validator, e.g. a copy of the same contact relayed to
        // us before the first one was stored, falls back to the general budget...
        assert!(budget.try_consume_refresh(&validator(1)));
        assert_eq!(budget.remaining(), (0, 2));

        // ...and is refused once that is gone, even though the reserve is not.
        assert!(!budget.try_consume_refresh(&validator(1)));
        assert_eq!(budget.remaining(), (0, 2));

        // Other validators still get their unit of the reserve.
        assert!(budget.try_consume_refresh(&validator(2)));
        assert_eq!(budget.remaining(), (0, 1));

        // The next tick lets every validator take a unit of the reserve again.
        budget.reset();
        assert!(budget.try_consume_refresh(&validator(1)));
        assert!(budget.try_consume_refresh(&validator(2)));
        assert_eq!(budget.remaining(), (1, 1));
    }

    #[test]
    fn concurrent_refreshes_of_one_validator_take_one_unit_of_the_reserve() {
        const THREADS: usize = 8;

        // Copies of the same refresh, checked on several discovery connections at once.
        for _ in 0..100 {
            let budget = ValidatorClaimBudget::with_refresh_reserve(0, THREADS);
            let barrier = std::sync::Barrier::new(THREADS);
            let admitted = std::thread::scope(|scope| {
                let threads: Vec<_> = (0..THREADS)
                    .map(|_| {
                        scope.spawn(|| {
                            barrier.wait();
                            budget.try_consume_refresh(&validator(1))
                        })
                    })
                    .collect();
                threads
                    .into_iter()
                    .map(|thread| thread.join().unwrap())
                    .filter(|&admitted| admitted)
                    .count()
            });
            assert_eq!(admitted, 1);
            assert_eq!(budget.remaining(), (0, THREADS - 1));
        }
    }

    #[test]
    fn the_general_budget_never_touches_the_reserve() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 1);

        assert!(budget.try_consume());
        assert!(!budget.try_consume());
        assert_eq!(budget.remaining(), (0, 1));

        assert!(budget.try_consume_refresh(&validator(1)));
        assert_eq!(budget.remaining(), (0, 0));
    }

    #[test]
    fn only_a_few_contacts_per_validator_verify_per_tick() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(2);
        let (first, second, third) = (PeerId::random(), PeerId::random(), PeerId::random());

        assert!(budget.may_verify(&validator(1), &first, 1));
        budget.record_verified(&validator(1), first, 1);
        // A peer verifies one contact per tick. Another copy of that one is fine, but not a
        // refresh, genuine or replayed, which would otherwise take up the share on its own.
        assert!(budget.may_verify(&validator(1), &first, 1));
        assert!(!budget.may_verify(&validator(1), &first, 2));
        assert!(!budget.may_verify(&validator(1), &first, 0));

        assert!(budget.may_verify(&validator(1), &second, 1));
        budget.record_verified(&validator(1), second, 1);

        // The share is used up for any further peer...
        assert!(!budget.may_verify(&validator(1), &third, 1));
        // ...but not for a contact that already took part of it.
        assert!(budget.may_verify(&validator(1), &second, 1));
        // Other validators keep their own share, and the general budget is left alone.
        assert!(budget.may_verify(&validator(2), &third, 1));
        assert_eq!(budget.remaining(), (10, 0));

        // The next tick restores the share.
        budget.reset();
        assert!(budget.may_verify(&validator(1), &third, 1));
        assert!(budget.may_verify(&validator(1), &first, 2));
    }

    #[test]
    fn a_zero_per_validator_limit_verifies_nothing() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 0).with_per_validator_limit(0);
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 0));
    }

    #[test]
    fn a_budget_without_a_per_validator_limit_verifies_any_number_of_contacts() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 0);
        for timestamp in 0..100 {
            budget.record_verified(&validator(1), PeerId::random(), timestamp);
        }
        assert!(budget.may_verify(&validator(1), &PeerId::random(), 0));
    }

    #[test]
    fn a_budget_without_a_reserve_is_only_the_general_budget() {
        let budget = ValidatorClaimBudget::new(1);
        assert_eq!(budget.remaining(), (1, 0));

        assert!(budget.try_consume_refresh(&validator(1)));
        assert!(!budget.try_consume());
        budget.reset();
        assert_eq!(budget.remaining(), (1, 0));
    }
}
