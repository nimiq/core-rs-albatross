use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
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
/// Independently of both counters, only a few contacts claiming the same validator address can
/// verify per tick (see [`Self::may_verify`]). A validator can sign claims for as many peer IDs as
/// it likes, and refresh each of them every second. Without this limit a single one could spend
/// the general budget of every node its contacts reach, since verified contacts are gossiped,
/// keeping other validators' first claims from being verified. Only claims that verify take a
/// place in this share, so bogus claims naming an honest validator's address cannot use it up.
///
/// A peer's first contact that verifies in a tick takes a place, however old it is. After that,
/// only a strictly newer contact of the same peer that is fresh, i.e. younger than
/// [`Self::with_fresh_contact_age`] on this node's clock, can verify, and it takes another place.
/// Validators re-sign their contact once per house-keeping tick, and no contact from the future is
/// stored, so at most one of a peer's regularly re-signed contacts is fresh at any time, apart from
/// a short overlap: a house-keeping tick that runs late, e.g. because of its re-check sweep, does
/// not delay the next one, and timestamps are whole seconds, so two consecutive contacts can be
/// less than a tick apart, and are then both fresh for about as long as the earlier re-sign ran
/// late, plus up to a second. Anybody can replay a peer's genuine older contacts, one after the
/// other and each newer than the last, but none of them is fresh, except for the one before the
/// current one during such an overlap. So together they take a single place, or two, and the peer's
/// current contact still verifies when it arrives after them, as long as it is fresh by then.
/// Contacts re-signed outside that schedule, e.g. when a node restarts or installs its validator
/// claim signer, are fresh as well, and can take further places.
///
/// Only claims checked on arrival are recorded in the share. The discovery behaviour's re-check
/// sweep keeps its own count of the same limit, so that a pending claim it verifies does not take
/// a place that the peer's next refresh, which arrives later in the same tick, may need. About
/// twice the limit of contacts claiming the same validator can thus verify per tick on this node.
/// The nodes their contacts are gossiped to are still bounded by their own shares.
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
    /// How many places each validator address has in the share of contacts that verify on arrival
    /// per tick.
    verified_claims_per_validator: usize,
    /// How young, in seconds, a contact must be to verify on arrival after an older contact of the
    /// same peer did in the same tick. 0 means that no contact is that young.
    fresh_contact_age: u64,
    /// For each validator address, the places of its share that claims verified on arrival took
    /// since the last reset.
    ///
    /// Entries are only added for claims that verified, i.e. that the validator signed, so this
    /// only holds addresses of validators that signed claims this tick. The limit is checked
    /// before a claim is checked and places are taken after, so claims checked at the same time
    /// on several discovery connections can exceed it by up to that many. That only costs a few
    /// more checks: bindings are capped per validator by the contact book anyway. Contacts of a
    /// peer that are checked at the same time as its first one, and are not fresh, share its
    /// place. The lock is never held while a claim is checked, nor while any other lock is taken.
    verified_claims: Mutex<HashMap<Address, VerifiedShare>>,
}

/// The places of one validator's share of contacts that verify on arrival (see
/// [`ValidatorClaimBudget::may_verify`]) that were taken since the last reset.
#[derive(Debug, Default)]
struct VerifiedShare {
    /// For each peer claiming the validator, the timestamp of its newest contact that verified.
    newest: HashMap<PeerId, u64>,
    /// The contacts that took a place, by peer and timestamp: the first contact of each peer that
    /// verified, and each later one that verified because it was fresh. A contact is in here only
    /// once, however many copies of it verified.
    places: HashSet<(PeerId, u64)>,
}

impl VerifiedShare {
    /// The places taken.
    fn taken(&self) -> usize {
        self.places.len()
    }
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
    /// It does not limit how many peers claiming the same validator can verify a contact per tick;
    /// see [`Self::with_per_validator_limit`]. No contact is fresh, so each of them verifies one
    /// contact per tick, and copies of it; see [`Self::with_fresh_contact_age`].
    pub fn with_refresh_reserve(capacity: usize, refresh_reserve: usize) -> Self {
        Self {
            capacity,
            refresh_reserve_capacity: refresh_reserve,
            remaining: AtomicUsize::new(capacity),
            remaining_refresh_reserve: AtomicUsize::new(refresh_reserve),
            refreshed_validators: Mutex::new(HashSet::new()),
            verified_claims_per_validator: usize::MAX,
            fresh_contact_age: 0,
            verified_claims: Mutex::new(HashMap::new()),
        }
    }

    /// Gives each validator address `verified_claims_per_validator` places in the share of
    /// contacts that verify on arrival per tick. See [`Self::may_verify`].
    pub fn with_per_validator_limit(mut self, verified_claims_per_validator: usize) -> Self {
        self.verified_claims_per_validator = verified_claims_per_validator;
        self
    }

    /// Lets a contact younger than `age` on this node's clock verify on arrival after an older
    /// contact of the same peer did in the same tick (see [`Self::may_verify`]).
    ///
    /// `age` must not exceed the interval at which peers re-sign their contact, so that, apart
    /// from a short overlap (see [`ValidatorClaimBudget`]), at most one of a peer's regularly
    /// re-signed contacts is fresh at any time, and replays of its older ones never are. A peer
    /// that re-signs its contact outside that schedule, e.g. because its node restarted or
    /// installed a validator claim signer, can have more fresh contacts.
    pub fn with_fresh_contact_age(mut self, age: Duration) -> Self {
        self.fresh_contact_age = age.as_secs();
        self
    }

    /// How many places each validator address has in the share of contacts that verify on arrival
    /// per tick. The re-check sweep applies the same limit to the claims it verifies, on its own.
    /// See [`Self::with_per_validator_limit`].
    pub fn verified_claims_per_validator(&self) -> usize {
        self.verified_claims_per_validator
    }

    /// Whether a contact timestamped `timestamp` is fresh at `unix_time`, in seconds: younger than
    /// the age set with [`Self::with_fresh_contact_age`]. A contact from the future is not.
    pub fn is_fresh(&self, timestamp: u64, unix_time: u64) -> bool {
        unix_time
            .checked_sub(timestamp)
            .is_some_and(|age| age < self.fresh_contact_age)
    }

    /// Whether the claim of the contact of `peer_id` timestamped `timestamp` to
    /// `validator_address` is worth checking this tick. That is the case if it is another copy of
    /// the newest contact of `peer_id` that verified on arrival this tick. Otherwise, the
    /// validator's share must have a place left, and either no contact of `peer_id` verified on
    /// arrival this tick, or `may_supersede` holds and this contact is newer than the one that did.
    ///
    /// On arrival, `may_supersede` is whether the contact [is fresh](Self::is_fresh), decided once
    /// and passed to [`Self::record_verified`] as well. The re-check sweep, which checks the stored
    /// contact of a peer and records nothing, passes `true`.
    ///
    /// A claim that is not worth checking should be left pending, without spending any budget on
    /// it. It is checked on a later tick, or superseded by a newer contact.
    pub fn may_verify(
        &self,
        validator_address: &Address,
        peer_id: &PeerId,
        timestamp: u64,
        may_supersede: bool,
    ) -> bool {
        let verified_claims = self.verified_claims.lock();
        let Some(share) = verified_claims.get(validator_address) else {
            return self.verified_claims_per_validator > 0;
        };
        match share.newest.get(peer_id) {
            Some(&newest) if newest == timestamp => true,
            _ if share.taken() >= self.verified_claims_per_validator => false,
            None => true,
            Some(&newest) => may_supersede && timestamp > newest,
        }
    }

    /// Records that the claim of the contact of `peer_id` timestamped `timestamp` to
    /// `validator_address` verified on arrival, with the `may_supersede` it was checked with. See
    /// [`Self::may_verify`].
    ///
    /// The first contact of a peer takes a place, and so does a later one that may supersede it,
    /// even if it is recorded after a newer one. A later one that may not, which was checked at
    /// the same time as the peer's first one, shares that one's place. A contact takes at most one
    /// place, however many copies of it are recorded.
    pub fn record_verified(
        &self,
        validator_address: &Address,
        peer_id: PeerId,
        timestamp: u64,
        may_supersede: bool,
    ) {
        let mut verified_claims = self.verified_claims.lock();
        let share = verified_claims
            .entry(validator_address.clone())
            .or_default();
        match share.newest.entry(peer_id) {
            Entry::Vacant(entry) => {
                entry.insert(timestamp);
                share.places.insert((peer_id, timestamp));
            }
            Entry::Occupied(entry) if *entry.get() == timestamp => {}
            Entry::Occupied(mut entry) => {
                if may_supersede {
                    share.places.insert((peer_id, timestamp));
                }
                let newest = entry.get_mut();
                *newest = (*newest).max(timestamp);
            }
        }
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
    /// validator take a unit of the reserve again, and frees every place of every validator's
    /// share of verified claims.
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

        assert!(budget.may_verify(&validator(1), &first, 1, false));
        budget.record_verified(&validator(1), first, 1, false);
        // A peer verifies one contact per tick, unless a later one may supersede it. Another copy
        // of that one is fine, but not an older or newer contact that may not, genuine or
        // replayed, which would otherwise take up the share on its own.
        assert!(budget.may_verify(&validator(1), &first, 1, false));
        assert!(!budget.may_verify(&validator(1), &first, 2, false));
        assert!(!budget.may_verify(&validator(1), &first, 0, false));
        // A newer one that may supersede it takes the place left, but an older one never does.
        assert!(budget.may_verify(&validator(1), &first, 2, true));
        assert!(!budget.may_verify(&validator(1), &first, 0, true));

        assert!(budget.may_verify(&validator(1), &second, 1, false));
        budget.record_verified(&validator(1), second, 1, false);

        // The share is used up for any further peer, or newer contact...
        assert!(!budget.may_verify(&validator(1), &third, 1, false));
        assert!(!budget.may_verify(&validator(1), &first, 2, true));
        // ...but not for a contact that already took part of it.
        assert!(budget.may_verify(&validator(1), &second, 1, false));
        // Other validators keep their own share, and the general budget is left alone.
        assert!(budget.may_verify(&validator(2), &third, 1, false));
        assert_eq!(budget.remaining(), (10, 0));

        // The next tick restores the share.
        budget.reset();
        assert!(budget.may_verify(&validator(1), &third, 1, false));
        assert!(budget.may_verify(&validator(1), &first, 2, false));
    }

    #[test]
    fn a_zero_per_validator_limit_verifies_nothing() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 0).with_per_validator_limit(0);
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 0, true));
    }

    #[test]
    fn a_budget_without_a_per_validator_limit_verifies_any_number_of_contacts() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 0);
        for timestamp in 0..100 {
            budget.record_verified(&validator(1), PeerId::random(), timestamp, false);
        }
        assert!(budget.may_verify(&validator(1), &PeerId::random(), 0, false));
    }

    /// Whether a contact of `peer_id` timestamped `timestamp` to validator 1 may verify, and if so,
    /// records that it did.
    fn verify(
        budget: &ValidatorClaimBudget,
        peer_id: PeerId,
        timestamp: u64,
        may_supersede: bool,
    ) -> bool {
        let admitted = budget.may_verify(&validator(1), &peer_id, timestamp, may_supersede);
        if admitted {
            budget.record_verified(&validator(1), peer_id, timestamp, may_supersede);
        }
        admitted
    }

    #[test]
    fn a_fresh_newer_contact_of_a_peer_takes_another_place() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(3);
        let (peer, second, third) = (PeerId::random(), PeerId::random(), PeerId::random());

        assert!(verify(&budget, peer, 100, false));
        // A newer contact verifies only if it may supersede the one that did.
        assert!(!budget.may_verify(&validator(1), &peer, 160, false));
        assert!(verify(&budget, peer, 160, true));
        // Older ones never do, and copies of the newest one take no place.
        assert!(!budget.may_verify(&validator(1), &peer, 100, true));
        assert!(verify(&budget, peer, 160, false));

        // So the share has one place left.
        assert!(verify(&budget, second, 100, false));
        assert!(!budget.may_verify(&validator(1), &third, 100, false));
        assert!(!budget.may_verify(&validator(1), &peer, 170, true));
        assert!(budget.may_verify(&validator(1), &peer, 160, false));
    }

    #[test]
    fn a_validator_verifies_at_most_its_share_of_fresh_contacts_of_one_peer() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(4);
        let peer = PeerId::random();

        let admitted: Vec<_> = (1..=10)
            .filter(|&timestamp| verify(&budget, peer, timestamp, true))
            .collect();
        assert_eq!(admitted, [1, 2, 3, 4]);
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 1, false));
        assert!(budget.may_verify(&validator(1), &peer, 4, false));
    }

    #[test]
    fn stale_contacts_of_a_peer_checked_at_once_share_its_place() {
        for ascending in [true, false] {
            let budget =
                ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(3);
            let peer = PeerId::random();

            // Both are checked before either is recorded.
            assert!(budget.may_verify(&validator(1), &peer, 100, false));
            assert!(budget.may_verify(&validator(1), &peer, 160, false));
            let timestamps = if ascending { [100, 160] } else { [160, 100] };
            for timestamp in timestamps {
                budget.record_verified(&validator(1), peer, timestamp, false);
            }

            // They took a single place, and the newest is the one a copy has to match.
            assert!(budget.may_verify(&validator(1), &peer, 160, false));
            assert!(verify(&budget, PeerId::random(), 100, false));
            assert!(verify(&budget, peer, 200, true));
            assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
        }
    }

    #[test]
    fn concurrent_stale_replays_of_one_peer_take_one_place() {
        const THREADS: u64 = 8;

        for _ in 0..100 {
            let budget =
                ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(2);
            let peer = PeerId::random();
            let barrier = std::sync::Barrier::new(THREADS as usize);
            std::thread::scope(|scope| {
                for timestamp in 0..THREADS {
                    let (budget, barrier) = (&budget, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        verify(budget, peer, 100 + timestamp, false);
                    });
                }
            });

            assert!(verify(&budget, PeerId::random(), 100, false));
            assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
        }
    }

    #[test]
    fn a_fresh_contact_recorded_after_a_newer_one_still_takes_a_place() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(4);
        let peer = PeerId::random();

        budget.record_verified(&validator(1), peer, 100, false);
        budget.record_verified(&validator(1), peer, 170, true);
        budget.record_verified(&validator(1), peer, 165, true);

        assert!(verify(&budget, PeerId::random(), 100, false));
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
        // The newest contact is the one kept.
        assert!(budget.may_verify(&validator(1), &peer, 170, false));
        assert!(!budget.may_verify(&validator(1), &peer, 165, true));
    }

    #[test]
    fn a_copy_of_the_newest_contact_takes_no_place() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(3);
        let peer = PeerId::random();

        budget.record_verified(&validator(1), peer, 100, false);
        budget.record_verified(&validator(1), peer, 100, false);
        budget.record_verified(&validator(1), peer, 160, true);
        budget.record_verified(&validator(1), peer, 160, true);

        assert!(verify(&budget, PeerId::random(), 100, false));
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
    }

    // Copies of a fresh contact can be checked on several connections at once, and recorded after
    // a newer contact of the same peer. However many copies are recorded, the contact takes a
    // single place.
    #[test]
    fn copies_of_a_contact_recorded_after_a_newer_one_take_a_single_place() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(4);
        let peer = PeerId::random();

        budget.record_verified(&validator(1), peer, 100, false);
        budget.record_verified(&validator(1), peer, 170, true);
        budget.record_verified(&validator(1), peer, 165, true);
        budget.record_verified(&validator(1), peer, 165, true);

        assert!(verify(&budget, PeerId::random(), 100, false));
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
    }

    // The same holds for a copy of the peer's first contact, which already took the peer's place.
    #[test]
    fn a_copy_of_a_peers_first_contact_recorded_after_a_newer_one_takes_no_place() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(3);
        let peer = PeerId::random();

        budget.record_verified(&validator(1), peer, 165, true);
        budget.record_verified(&validator(1), peer, 170, true);
        budget.record_verified(&validator(1), peer, 165, true);

        assert!(verify(&budget, PeerId::random(), 100, false));
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
    }

    // A fresh contact checked at the same time as a newer one of the same peer, which is recorded
    // first and so takes the peer's place, still takes a place of its own, and its copies none.
    #[test]
    fn a_fresh_contact_recorded_after_a_newer_first_one_takes_a_single_place() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(3);
        let peer = PeerId::random();

        budget.record_verified(&validator(1), peer, 170, true);
        budget.record_verified(&validator(1), peer, 165, true);
        budget.record_verified(&validator(1), peer, 165, true);

        assert!(verify(&budget, PeerId::random(), 100, false));
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));
    }

    #[test]
    fn freshness_is_exclusive_and_excludes_the_future() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 0)
            .with_fresh_contact_age(Duration::from_secs(60));
        assert!(budget.is_fresh(1000, 1000));
        assert!(budget.is_fresh(1000, 1059));
        assert!(!budget.is_fresh(1000, 1060));
        assert!(!budget.is_fresh(1001, 1000));

        // Without an age set, no contact is fresh.
        assert!(!ValidatorClaimBudget::new(1).is_fresh(1000, 1000));
    }

    #[test]
    fn reset_frees_the_places_taken_by_superseding_contacts() {
        let budget = ValidatorClaimBudget::with_refresh_reserve(10, 0).with_per_validator_limit(2);
        let peer = PeerId::random();
        assert!(verify(&budget, peer, 100, false));
        assert!(verify(&budget, peer, 160, true));
        assert!(!budget.may_verify(&validator(1), &PeerId::random(), 100, false));

        budget.reset();
        assert!(verify(&budget, PeerId::random(), 100, false));
        assert!(verify(&budget, PeerId::random(), 100, false));
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
