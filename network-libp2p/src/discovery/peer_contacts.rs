use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use instant::SystemTime;
use libp2p::{
    gossipsub,
    identity::{Keypair, PublicKey},
    multiaddr::Protocol,
    Multiaddr, PeerId,
};
use nimiq_keys::{Address, Ed25519Signature, KeyPair};
use nimiq_network_interface::{
    network::Network as NetworkInterface,
    peer_info::{PeerInfo, Services},
    validator_claim::{ValidatorClaim, ValidatorClaimSigner},
};
use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSignable, TaggedSignature, TaggedSigned};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::validator_verifier::{
    InvalidReason, SignedValidatorClaim, ValidatorClaimBudget, ValidatorClaimVerifier,
    ValidatorVerification,
};
use crate::{utils, Network};

#[derive(Debug, Error)]
pub enum PeerContactError {
    #[error("Exceeded number of advertised addresses")]
    AdvertisedAddressesExceeded,
}

/// Whether a contact timestamped `timestamp` (seconds since the epoch) exceeds `max_age` as of
/// `unix_time`. Future timestamps are treated as exceeded, to prevent immortal entries from
/// untrusted peers.
fn contact_exceeds_age(timestamp: u64, max_age: Duration, unix_time: Duration) -> bool {
    unix_time
        .checked_sub(Duration::from_secs(timestamp))
        .is_none_or(|age| age > max_age)
}

/// The validator info contains all information which is not present in a [PeerContact]
/// such that a [ValidatorClaim] can be constructed. Importantly this also includes the signature.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatorInfo {
    /// The address of the validator. This is the unique identifier for a validator.
    validator_address: Address,

    /// The signature for the [ValidatorClaim].
    /// It does _not_ verify for this structure, but only once the [nimiq_utils::tagged_signing::TaggedSigned] is reconstructed
    /// with the given information of this struct and the corresponding [PeerContact].
    signature: TaggedSignature<ValidatorClaim<<Network as NetworkInterface>::PeerId>, KeyPair>,
}

impl ValidatorInfo {
    pub fn new(
        validator_address: Address,
        signature: TaggedSignature<ValidatorClaim<<Network as NetworkInterface>::PeerId>, KeyPair>,
    ) -> Self {
        Self {
            validator_address,
            signature,
        }
    }

    /// The validator address claimed by this info. The claim is not verified here.
    pub fn validator_address(&self) -> &Address {
        &self.validator_address
    }

    /// Whether the signature has the size of an Ed25519 signature. A signature of any other size
    /// can never verify, so the claim is bogus without looking up the validator's signing key.
    fn has_well_formed_signature(&self) -> bool {
        self.signature.as_bytes().len() == Ed25519Signature::SIZE
    }
}

/// A plain peer contact. This contains:
///
///  - A set of multi-addresses for the peer.
///  - The peer's public key.
///  - A bitmask of the services supported by this peer.
///  - A timestamp when this contact information was generated.
///
/// Note that seed nodes are being tracked elsewhere.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerContact {
    /// Addresses that we advertise
    pub addresses: Vec<Multiaddr>,

    /// Public key of this peer.
    #[serde(with = "self::serde_public_key")]
    pub public_key: PublicKey,

    /// Services supported by this peer.
    pub services: Services,

    /// Optional [ValidatorInfo] in case the node represented by this contact is
    /// running a validator.
    validator_info: Option<ValidatorInfo>,

    /// Timestamp when this peer contact was created in *seconds* since unix epoch.
    timestamp: u64,
}

impl PeerContact {
    /// Maximum number of advertised addresses
    const MAX_ADDRESSES: usize = 15;

    pub fn new<I: IntoIterator<Item = Multiaddr>>(
        advertised_addresses: I,
        public_key: PublicKey,
        services: Services,
        timestamp: u64,
    ) -> Result<Self, PeerContactError> {
        let mut addresses = advertised_addresses.into_iter().collect::<Vec<Multiaddr>>();
        if addresses.len() > Self::MAX_ADDRESSES {
            return Err(PeerContactError::AdvertisedAddressesExceeded);
        }

        addresses.sort();

        Ok(Self {
            addresses,
            public_key,
            services,
            validator_info: None,
            timestamp,
        })
    }

    /// Derives the peer ID from the public key
    pub fn peer_id(&self) -> PeerId {
        self.public_key.clone().to_peer_id()
    }

    /// Returns the timestamp of the contact. It is generally set to the time the contact was created.
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Signs this peer contact.
    ///
    /// # Panics
    ///
    /// This panics if the peer contacts public key doesn't match the supplied key pair.
    ///
    pub fn sign(self, keypair: &Keypair) -> SignedPeerContact {
        if keypair.public() != self.public_key {
            panic!("Supplied keypair doesn't match the public key in the peer contact.");
        }

        let signature = keypair.tagged_sign(&self);

        SignedPeerContact {
            inner: self,
            signature,
        }
    }

    /// This sets the timestamp in the peer contact to the current system time.
    pub fn set_current_time(&mut self) {
        self.timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// Adds a set of addresses
    pub fn add_addresses(&mut self, addresses: Vec<Multiaddr>) {
        if addresses.len() + self.addresses.len() > Self::MAX_ADDRESSES {
            log::warn!(
                maximum = Self::MAX_ADDRESSES,
                ignored_addresses = ?addresses[Self::MAX_ADDRESSES - self.addresses.len()..],
                "Ignoring some of the addresses since it exceeds the maximum allowed"
            );
        }
        self.addresses.extend(
            addresses
                .into_iter()
                .take(Self::MAX_ADDRESSES - self.addresses.len())
                .collect::<Vec<Multiaddr>>(),
        );
    }

    /// Removes addresses
    pub fn remove_addresses(&mut self, addresses: Vec<Multiaddr>) {
        let to_remove_addresses: HashSet<Multiaddr> = HashSet::from_iter(addresses);
        self.addresses
            .retain(|addr| !to_remove_addresses.contains(addr));
    }

    /// Verifies whether the lengths of the advertised addresses are within
    /// the expected limits. This is helpful to verify a received peer contact.
    pub fn verify(&self) -> Result<(), PeerContactError> {
        if self.addresses.len() > Self::MAX_ADDRESSES {
            return Err(PeerContactError::AdvertisedAddressesExceeded);
        }
        Ok(())
    }

    /// The [`ValidatorInfo`] claimed by this contact, if any. The claim is not verified here.
    pub fn validator_info(&self) -> Option<&ValidatorInfo> {
        self.validator_info.as_ref()
    }

    /// The validator address claimed by this contact, if any. The claim is not verified here.
    pub fn validator_address(&self) -> Option<&Address> {
        self.validator_info
            .as_ref()
            .map(ValidatorInfo::validator_address)
    }

    /// Attaches (or removes) the validator claim of this contact.
    ///
    /// This invalidates any existing signature over the contact, so it must be called before
    /// signing.
    pub fn set_validator_info(&mut self, validator_info: Option<ValidatorInfo>) {
        self.validator_info = validator_info;
    }

    /// Reconstructs the signed [`ValidatorClaim`] of this contact, if any.
    ///
    /// The claim binds the contact's peer ID and timestamp to the validator address, so it is
    /// only meaningful together with the contact it was taken from.
    pub fn signed_validator_claim(&self) -> Option<SignedValidatorClaim> {
        let validator_info = self.validator_info.as_ref()?;
        let claim = ValidatorClaim::new(
            self.peer_id(),
            validator_info.validator_address.clone(),
            self.timestamp,
        );
        Some(TaggedSigned::new(claim, validator_info.signature.clone()))
    }
}

impl TaggedSignable for PeerContact {
    const TAG: u8 = 0x02;
}

/// A signed peer contact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedPeerContact {
    /// The wrapped peer contact.
    pub inner: PeerContact,

    /// The signature over the serialized peer contact.
    pub signature: TaggedSignature<PeerContact, Keypair>,
}

impl SignedPeerContact {
    /// Verifies that the signature is valid for this peer contact and also does
    /// intrinsic verification on the inner PeerContact.
    pub fn verify(&self) -> bool {
        if self.inner.verify().is_err() {
            return false;
        };
        self.signature
            .tagged_verify(&self.inner, &self.inner.public_key)
    }

    /// Gets the public key of this peer contact.
    pub fn public_key(&self) -> &PublicKey {
        &self.inner.public_key
    }

    /// Gets the Peer ID that results from this peer contact's peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }

    /// Checks the validator claim carried by this contact, returning `None` if it makes none.
    ///
    /// This is pure and takes no contact book locks, so it can (and must) be called before
    /// inserting the contact into the book.
    ///
    /// A claim whose signature does not even have the size of one is rejected without asking
    /// `verifier`, which would look up the validator's signing key first.
    pub fn check_validator_claim(
        &self,
        verifier: &dyn ValidatorClaimVerifier,
    ) -> Option<ValidatorVerification> {
        if !self.inner.validator_info()?.has_well_formed_signature() {
            return Some(ValidatorVerification::Invalid(
                InvalidReason::InvalidSignature,
            ));
        }
        let signed_claim = self.inner.signed_validator_claim()?;
        Some(verifier.verify_validator_claim(&signed_claim))
    }

    /// Whether this contact carries a validator claim whose signature is malformed, so that it can
    /// be rejected without spending any budget on it.
    fn has_malformed_validator_claim(&self) -> bool {
        self.inner
            .validator_info()
            .is_some_and(|info| !info.has_well_formed_signature())
    }
}

/// The verification status of a [`SignedPeerContact`]'s validator claim.
///
/// Contacts received from peers get it from [`CheckedPeerContact::check_new_with_budget`], which
/// checks the claim only if the contact is new to the book and the claim budget allows it, and
/// otherwise leaves it [`Pending`](Self::Pending) unchecked, as [`CheckedPeerContact::pending`]
/// does. The pending claim of a stored contact is settled later by the periodic re-check sweep
/// (see [`PeerContactBook::unverified_validator_contacts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimCheck {
    /// This contact carries no validator claim.
    None,
    /// The claim was cryptographically checked against this exact contact and holds.
    Verified,
    /// The claim was cryptographically checked against this exact contact and does not hold.
    Invalid,
    /// This exact contact's claim was not conclusively checked: budget was unavailable, or the
    /// check came back [`ValidatorVerification::Unverifiable`]. The contact stays unverified
    /// until the periodic re-check sweep reaches a conclusive result.
    Pending,
}

/// A [`SignedPeerContact`] together with the outcome of checking its validator claim.
///
/// Only a conclusive verification can establish or refresh a validator binding. A pending refresh
/// that keeps the same validator address leaves the previous binding in place, with its original
/// timestamp and expiry. Converting a plain [`SignedPeerContact`] leaves its claim, if it has one,
/// pending, just like [`CheckedPeerContact::pending`].
#[derive(Clone, Debug)]
pub struct CheckedPeerContact {
    contact: SignedPeerContact,
    claim: ClaimCheck,
}

impl CheckedPeerContact {
    /// Checks the contact's validator claim, if it has one, without charging any budget.
    ///
    /// Only used by tests; see [`ClaimCheck`] for how claims are checked in production.
    #[cfg(test)]
    pub(crate) fn check(contact: SignedPeerContact, verifier: &dyn ValidatorClaimVerifier) -> Self {
        Self::check_with_outcome(contact, verifier).0
    }

    /// Checks the contact's validator claim, if it has one, and also returns the raw outcome of
    /// the check, which tells *why* a claim was left pending. `None` if the contact carries no
    /// claim.
    fn check_with_outcome(
        contact: SignedPeerContact,
        verifier: &dyn ValidatorClaimVerifier,
    ) -> (Self, Option<ValidatorVerification>) {
        let verification = contact.check_validator_claim(verifier);
        let claim = match verification {
            None => ClaimCheck::None,
            Some(ValidatorVerification::Verified) => ClaimCheck::Verified,
            Some(ValidatorVerification::Invalid(reason)) => {
                debug!(
                    peer_id = %contact.peer_id(),
                    ?reason,
                    "Ignoring bogus validator claim in peer contact",
                );
                ClaimCheck::Invalid
            }
            // Inconclusive: we simply don't know yet (e.g. our own staking-contract state is
            // incomplete). This must never be treated as a definitive rejection.
            Some(ValidatorVerification::Unverifiable(_)) => ClaimCheck::Pending,
        };

        (Self { contact, claim }, verification)
    }

    /// The underlying signed contact.
    pub fn signed(&self) -> &SignedPeerContact {
        &self.contact
    }

    /// Checks the contact's validator claim if it has one and the general `budget` allows it;
    /// otherwise leaves it pending rather than actually checked. Like
    /// [`Self::check_new_with_budget`], it exhausts `budget` if the check comes back unverifiable
    /// for a node-wide reason.
    ///
    /// Only used by tests. In production, contacts received from peers (in the discovery
    /// handshake and peer-address updates) go through [`Self::check_new_with_budget`], and the
    /// claims it leaves pending are settled by the periodic re-check sweep (see
    /// [`PeerContactBook::unverified_validator_contacts`]). Unlike [`Self::check_new_with_budget`],
    /// this does not look at the contact book, so it spends the budget even on contacts the book
    /// would discard, and it never spends the refresh reserve.
    #[cfg(test)]
    pub(crate) fn check_with_budget(
        contact: SignedPeerContact,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
    ) -> Self {
        Self::check_charging(contact, verifier, budget, ValidatorClaimBudget::try_consume)
    }

    /// Checks the validator claim of a contact received from a peer, if it has one and `budget`
    /// allows it; otherwise leaves it pending rather than actually checked.
    ///
    /// Verifying a claim reads blockchain state, so on hot, attacker-reachable paths (the
    /// discovery handshake and periodic peer-address updates) the number of checks performed
    /// must be bounded regardless of how many connections present a claim. A contact left
    /// pending here is picked up later by [`PeerContactBook::unverified_validator_contacts`].
    ///
    /// If the check comes back unverifiable for a node-wide reason (see
    /// [`UnverifiableReason::is_node_wide`](super::validator_verifier::UnverifiableReason::is_node_wide)),
    /// e.g. because this node is still syncing its staking contract, every other claim would too,
    /// so the whole budget is exhausted until the next house-keeping tick instead of spending it on
    /// blockchain reads that cannot succeed.
    ///
    /// A contact that `book` would discard anyway, because it is our own, from the future, or not
    /// newer than the one we have for its peer (e.g. the same contact relayed to us by several
    /// peers), is not worth spending the budget on (see [`PeerContactBook::is_new`]). Its claim is
    /// left pending, so that the re-check sweep picks it up in case it does get stored after all.
    ///
    /// A pending refresh that keeps the same validator address leaves a previous verified binding
    /// in place, but cannot refresh its timestamp or make the new contact gossipable. A pending
    /// refresh that claims a different address removes the previous binding. See
    /// [`PeerContactBook::store`].
    ///
    /// A new contact whose peer already holds a live verified binding to the validator address it
    /// claims (see [`PeerContactBook::has_live_verified_binding`]) is charged to the budget's
    /// refresh reserve first, and to the general budget only once the reserve is used up or that
    /// validator already took its one unit of the reserve this tick (see
    /// [`ValidatorClaimBudget::try_consume_refresh`]). Any other contact is charged to the general
    /// budget only. Getting such a binding takes the validator's signing key, and relaying the
    /// validator's genuine refreshed contact to us at most spends that validator's unit on
    /// verifying it, so unauthenticated peers flooding the general budget with bogus claims cannot
    /// keep an honest validator's refreshed contact from being verified, and thereby gossiped.
    ///
    /// Copies of that same contact checked before the first one is stored, e.g. relayed on
    /// several connections at once, fall back to the general budget and may thus be left pending.
    /// If such a copy is stored before the one that was checked, the checked one still settles
    /// the claim once it is stored; see [`PeerContactBook::store`].
    ///
    /// A claim is only checked while the validator it claims has not used up its share of
    /// verified claims for this tick (see [`ValidatorClaimBudget::may_verify`]); otherwise it is
    /// left pending without spending any budget. If it verifies, it takes part of that share. A
    /// validator can sign claims for as many peer IDs as it likes, and refresh each of them every
    /// second, and this keeps a single one from spending the general budget of every node its
    /// contacts reach.
    ///
    /// A claim whose signature is malformed is rejected without spending any budget, since it can
    /// be rejected without reading blockchain state.
    ///
    /// The book is only read briefly, once to tell whether the contact is new and whether its peer
    /// holds such a binding, and never locked while the claim is checked, since checking reads
    /// blockchain state.
    ///
    /// This is for a contact that is inserted with [`PeerContactBook::insert`]. For contacts that
    /// are inserted with [`PeerContactBook::insert_filtered`], use
    /// [`Self::check_new_filtered_with_budget`].
    pub fn check_new_with_budget(
        contact: SignedPeerContact,
        book: &RwLock<PeerContactBook>,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
    ) -> Self {
        Self::check_if_stored_with_budget(contact, book, verifier, budget, None)
    }

    /// Like [`Self::check_new_with_budget`], for a contact that is inserted with
    /// [`PeerContactBook::insert_filtered`] and `filter`: a contact that the filter would discard,
    /// e.g. because it is too old or lacks the services we need, is not worth spending the budget
    /// on either.
    pub fn check_new_filtered_with_budget(
        contact: SignedPeerContact,
        book: &RwLock<PeerContactBook>,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
        filter: &InsertFilter,
    ) -> Self {
        Self::check_if_stored_with_budget(contact, book, verifier, budget, Some(filter))
    }

    fn check_if_stored_with_budget(
        contact: SignedPeerContact,
        book: &RwLock<PeerContactBook>,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
        filter: Option<&InsertFilter>,
    ) -> Self {
        let (would_store, refreshed_validator) = {
            let book = book.read();
            let would_store = book.would_store(&contact, filter);
            let refreshed_validator = contact
                .inner
                .validator_address()
                .filter(|&address| {
                    would_store && book.has_live_verified_binding(&contact.peer_id(), address)
                })
                .cloned();
            (would_store, refreshed_validator)
        };

        if !would_store {
            return Self::pending(contact);
        }

        // Only check the claim while the validator it claims has not used up its share of
        // verified claims for this tick.
        let claimed = contact
            .inner
            .validator_address()
            .map(|address| (address.clone(), contact.peer_id(), contact.inner.timestamp));
        if let Some((address, peer_id, timestamp)) = &claimed
            && !contact.has_malformed_validator_claim()
            && !budget.may_verify(address, peer_id, *timestamp)
        {
            return Self::pending(contact);
        }

        let checked =
            Self::check_charging(
                contact,
                verifier,
                budget,
                |budget| match &refreshed_validator {
                    Some(validator_address) => budget.try_consume_refresh(validator_address),
                    None => budget.try_consume(),
                },
            );
        if let Some((address, peer_id, timestamp)) = claimed
            && checked.claim == ClaimCheck::Verified
        {
            budget.record_verified(&address, peer_id, timestamp);
        }
        checked
    }

    /// Checks the contact's validator claim if it has one and `consume` takes a unit of `budget`
    /// for it; otherwise leaves it pending. Exhausts `budget` if the check shows that no claim can
    /// be verified by this node right now. A claim whose signature is malformed is rejected
    /// without charging `budget`.
    fn check_charging(
        contact: SignedPeerContact,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
        consume: impl FnOnce(&ValidatorClaimBudget) -> bool,
    ) -> Self {
        if contact.has_malformed_validator_claim() {
            return Self::check_with_outcome(contact, verifier).0;
        }
        if contact.inner.validator_info().is_none() || !consume(budget) {
            return Self::pending(contact);
        }
        let (checked, verification) = Self::check_with_outcome(contact, verifier);
        if verification.is_some_and(|verification| verification.is_node_wide_unverifiable()) {
            debug!(
                ?verification,
                "Cannot verify validator claims right now, skipping checks until next tick",
            );
            budget.exhaust();
        }
        checked
    }

    /// Leaves the contact's validator claim, if it has one, pending without checking it.
    pub fn pending(contact: SignedPeerContact) -> Self {
        let claim = if contact.inner.validator_info().is_some() {
            ClaimCheck::Pending
        } else {
            ClaimCheck::None
        };
        Self { contact, claim }
    }
}

impl From<SignedPeerContact> for CheckedPeerContact {
    /// Leaves the contact's validator claim, if it has one, pending without checking it.
    ///
    /// Treating a claim as absent instead would store it as settled, so the re-check sweep would
    /// never look at it, and it would end the peer's existing binding.
    fn from(contact: SignedPeerContact) -> Self {
        Self::pending(contact)
    }
}

/// Keeps, of the contacts in one batch received from a peer, only the newest one of each peer,
/// in the order they came in. Contacts from the future are dropped.
///
/// The contact book would only ever store that one: it discards contacts from the future and
/// contacts that are not newer than the one it has. Checking the claims of the others, e.g. of
/// several copies of the same contact, would only spend the claim budget, since every contact in
/// a batch is checked before any of them is stored. If the newest one then does not pass the
/// filter of [`PeerContactBook::insert_filtered`], none of them is stored, even if an older one
/// would have passed; that older one is superseded anyway.
pub fn newest_contact_per_peer(contacts: Vec<SignedPeerContact>) -> Vec<SignedPeerContact> {
    let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
        return contacts;
    };
    let now = unix_time.as_secs();

    // For each peer, the position and timestamp of its newest contact. Of equally new ones, the
    // first is kept.
    let mut newest: HashMap<PeerId, (usize, u64)> = HashMap::new();
    for (position, contact) in contacts.iter().enumerate() {
        let timestamp = contact.inner.timestamp;
        if timestamp > now {
            continue;
        }
        match newest.entry(contact.peer_id()) {
            Entry::Occupied(mut entry) => {
                if timestamp > entry.get().1 {
                    entry.insert((position, timestamp));
                }
            }
            Entry::Vacant(entry) => {
                entry.insert((position, timestamp));
            }
        }
    }

    let keep: HashSet<usize> = newest.into_values().map(|(position, _)| position).collect();
    contacts
        .into_iter()
        .enumerate()
        .filter_map(|(position, contact)| keep.contains(&position).then_some(contact))
        .collect()
}

/// The filter that [`PeerContactBook::insert_filtered`] applies to contacts relayed to us.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsertFilter {
    /// The services a contact must provide, unless both we and the contact are validators.
    pub services: Services,
    /// Whether a contact must advertise a secure websocket address.
    pub only_secure_ws_connections: bool,
}

/// Meta information attached to peer contact info objects. This is meant to be mutable and change over time.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PeerContactMeta {
    outer_protocol_address: Option<Multiaddr>,
    score: f64,
    /// Whether this exact contact's validator claim was conclusively verified.
    validator_verified: bool,
    /// Whether this contact still needs a conclusive check: verification was skipped for budget
    /// or returned [`ValidatorVerification::Unverifiable`]. Both verified and invalid claims are
    /// excluded from the periodic re-check sweep.
    validator_verification_pending: bool,
    /// When the re-check sweep last picked the stale binding of this contact's peer for a
    /// re-check, in seconds since the Unix epoch. See
    /// [`PeerContactBook::stale_verified_validator_contacts`].
    binding_rechecked_at: Option<u64>,
}

/// This encapsulates a peer contact (signed), but also pre-computes frequently used values such as `peer_id` and
/// `protocols`. It also contains meta-data that can be mutated.
#[derive(Debug)]
pub struct PeerContactInfo {
    /// The peer ID derived from the public key in the peer contact.
    peer_id: PeerId,

    /// The peer contact data with signature.
    contact: SignedPeerContact,

    /// Mutable meta-data.
    meta: RwLock<PeerContactMeta>,
}

impl From<SignedPeerContact> for PeerContactInfo {
    fn from(contact: SignedPeerContact) -> Self {
        Self::new(contact, false, false)
    }
}

impl PeerContactInfo {
    /// Constructs contact info with the verification status of this exact contact's claim.
    fn new(contact: SignedPeerContact, validator_verified: bool, pending: bool) -> Self {
        let peer_id = contact.inner.peer_id();
        Self {
            peer_id,
            contact,
            meta: RwLock::new(PeerContactMeta {
                score: 0.,
                outer_protocol_address: None,
                validator_verified,
                validator_verification_pending: pending,
                binding_rechecked_at: None,
            }),
        }
    }

    /// Short-hand for the plain [`PeerContact`]
    pub fn contact(&self) -> &PeerContact {
        &self.contact.inner
    }

    /// Short-hand for the signed [`SignedPeerContact`]
    pub fn signed(&self) -> &SignedPeerContact {
        &self.contact
    }

    /// Returns the peer ID of this contact.
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// Returns the supported services of this contact.
    pub fn services(&self) -> Services {
        self.contact.inner.services
    }

    /// Returns the public key of this contact.
    pub fn public_key(&self) -> &PublicKey {
        &self.contact.inner.public_key
    }

    /// Returns an iterator over the observed multi-addresses of this contact.
    pub fn addresses(&self) -> impl Iterator<Item = &Multiaddr> {
        self.contact.inner.addresses.iter()
    }

    /// Returns whether the peer contact exceeds its age limit
    pub fn exceeds_age(&self, max_age: Duration, unix_time: Duration) -> bool {
        contact_exceeds_age(self.contact.inner.timestamp(), max_age, unix_time)
    }

    /// Returns true if the services provided are interesting to me
    pub fn matches(&self, services: Services) -> bool {
        self.services().contains(services)
    }

    /// Gets the peer score
    pub fn get_score(&self) -> f64 {
        self.meta.read().score
    }

    /// Sets the peer score
    pub fn set_score(&self, score: f64) {
        self.meta.write().score = score;
    }

    /// Gets the outer protocol address of the peer. For example `/ip4/x.x.x.x` or `/dns4/foo.bar`
    pub fn get_outer_protocol_address(&self) -> Option<Multiaddr> {
        self.meta.read().outer_protocol_address.clone()
    }

    /// The validator address claimed by this contact, if any.
    ///
    /// The claim may be unverified; use [`PeerContactInfo::is_validator_verified`] to tell.
    pub fn validator_address(&self) -> Option<&Address> {
        self.contact.inner.validator_address()
    }

    /// Whether this exact contact's validator claim was checked and holds.
    pub fn is_validator_verified(&self) -> bool {
        self.meta.read().validator_verified
    }

    /// Whether this contact's validator claim should still be offered to the periodic re-check
    /// sweep ([`PeerContactBook::unverified_validator_contacts`]): this exact contact's claim has
    /// not been conclusively checked yet, because the check was skipped for budget or came back
    /// [`ValidatorVerification::Unverifiable`].
    ///
    /// A claim that *was* conclusively checked is not re-offered, whether the outcome was
    /// [`ValidatorVerification::Verified`] or [`ValidatorVerification::Invalid`]. This matters for
    /// `Invalid`: the reconstructed claim binds this contact's exact peer ID and timestamp, so a
    /// signature that does not verify against the validator's on-chain signing key (or an address
    /// that is not a validator) can never start verifying for *this* contact. Re-offering it would
    /// let a peer that attaches a bogus claim to every contact it gossips permanently occupy the
    /// sweep and burn its bounded per-tick blockchain-read budget on known-bad claims, starving the
    /// re-check of claims that legitimately came back `Unverifiable`. The contact stays stored and
    /// dialable; only its rejected claim is left alone until the peer advertises a fresh contact,
    /// which is checked anew on arrival.
    pub(crate) fn validator_claim_needs_recheck(&self) -> bool {
        self.meta.read().validator_verification_pending
    }

    /// Whether this contact may be passed on to other peers.
    ///
    /// Contacts carrying a validator claim we could not verify are still stored and dialed, but
    /// never gossiped, so that we do not spread claims we cannot vouch for.
    pub fn is_gossipable(&self) -> bool {
        self.validator_address().is_none() || self.is_validator_verified()
    }

    /// Records a definitive verification outcome for the current contact: `verified` reflects
    /// the result, and the claim is no longer pending re-check.
    pub(crate) fn set_validator_verified(&self, verified: bool) {
        let mut meta = self.meta.write();
        meta.validator_verified = verified;
        meta.validator_verification_pending = false;
    }

    /// Notes that the re-check sweep picked the stale binding of this contact's peer for a
    /// re-check at `unix_time`, so that it is not picked again before [`STALE_BINDING_AGE`]
    /// passed.
    ///
    /// [`STALE_BINDING_AGE`]: PeerContactBook::STALE_BINDING_AGE
    pub(crate) fn mark_binding_rechecked(&self, unix_time: Duration) {
        self.meta.write().binding_rechecked_at = Some(unix_time.as_secs());
    }

    /// Whether the stale binding of this contact's peer was picked for a re-check less than
    /// [`PeerContactBook::STALE_BINDING_AGE`] before `unix_time`.
    fn binding_rechecked_recently(&self, unix_time: Duration) -> bool {
        self.meta
            .read()
            .binding_rechecked_at
            .is_some_and(|rechecked_at| {
                !contact_exceeds_age(
                    rechecked_at,
                    Duration::from_secs(PeerContactBook::STALE_BINDING_AGE),
                    unix_time,
                )
            })
    }

    /// Sets the outer protocol address of the peer once
    pub fn set_outer_protocol_address(&self, addr: Multiaddr) {
        self.meta
            .write()
            .outer_protocol_address
            .get_or_insert_with(|| {
                trace!(peer_id = %self.peer_id, %addr, "Set outer protocol address for peer");
                addr
            });
    }
}

/// Main structure that holds the peer information that has been obtained or
/// discovered by the discovery protocol.
#[derive(Debug)]
pub struct PeerContactBook {
    /// Contact information for our own.
    own_peer_contact: PeerContactInfo,
    /// Own Peer ID (also present in `own_peer_contact`)
    own_peer_id: PeerId,
    /// Contact information for other peers in the network indexed by their
    /// peer ID.
    peer_contacts: HashMap<PeerId, Arc<PeerContactInfo>>,
    /// Verified validator bindings: for each validator address and peer ID, the timestamp of that
    /// peer's last contact whose claim to the address was conclusively verified.
    ///
    /// The timestamp sets both the binding's priority and its expiry. A pending refresh with the
    /// same address keeps the binding but cannot change its timestamp. The latest contact in
    /// `peer_contacts` determines gossip eligibility, and must still claim the same validator
    /// address for the binding to be reported. This never contains our own peer ID.
    verified_claim_timestamps: HashMap<Address, HashMap<PeerId, u64>>,
    /// Signs the validator claim attached to our own contact, if we run a registered validator.
    validator_claim_signer: Option<ValidatorClaimSigner>,
    /// Only return secure websocket addresses.
    /// With this flag non secure websocket addresses will be stored (to still have a valid signature of the peer contact)
    /// but won't be returned when calling `get_addresses`
    only_secure_addresses: bool,
    /// Flag to indicate whether to return also loopback addresses
    allow_loopback_addresses: bool,
    /// Flag to indicate whether to support memory transport addresses
    memory_transport: bool,
}

impl PeerContactBook {
    /// If a peer's age exceeds this value in seconds, it is removed (30 minutes)
    pub const MAX_PEER_AGE: u64 = 30 * 60;

    /// The maximum number of live verified bindings kept for a validator address. See
    /// [`Self::index_add`].
    pub const MAX_BINDINGS_PER_VALIDATOR: usize = 4;

    /// A verified binding whose contact is older than this, in seconds, is re-checked against the
    /// staking contract by the re-check sweep (5 minutes). See
    /// [`Self::stale_verified_validator_contacts`].
    ///
    /// An online validator re-signs its contact on every house-keeping tick, and each refreshed
    /// contact is checked anew, so this only concerns bindings that stopped being refreshed, e.g.
    /// by someone holding a signing key that was rotated away since.
    pub const STALE_BINDING_AGE: u64 = 5 * 60;

    /// Creates a new `PeerContactBook` given our own peer contact information.
    pub fn new(
        own_peer_contact: SignedPeerContact,
        only_secure_addresses: bool,
        allow_loopback_addresses: bool,
        memory_transport: bool,
    ) -> Self {
        let own_peer_id = own_peer_contact.inner.peer_id();
        Self {
            own_peer_contact: own_peer_contact.into(),
            own_peer_id,
            peer_contacts: HashMap::new(),
            verified_claim_timestamps: HashMap::new(),
            validator_claim_signer: None,
            only_secure_addresses,
            allow_loopback_addresses,
            memory_transport,
        }
    }

    /// Whether storing `contact` could change anything: it is not our own contact, not from the
    /// future, and newer than the one we have for its peer, if any. Anything else is discarded on
    /// insertion.
    pub fn is_new(&self, contact: &SignedPeerContact) -> bool {
        let peer_id = contact.peer_id();
        let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
            return true;
        };
        peer_id != self.own_peer_id
            && contact.inner.timestamp <= unix_time.as_secs()
            && self
                .peer_contacts
                .get(&peer_id)
                .is_none_or(|existing| existing.contact().timestamp < contact.inner.timestamp)
    }

    /// Whether inserting `contact` could change anything: it [is new](Self::is_new) and, if it is
    /// inserted with [`Self::insert_filtered`] and `filter`, passes that filter.
    pub fn would_store(&self, contact: &SignedPeerContact, filter: Option<&InsertFilter>) -> bool {
        self.is_new(contact)
            && filter.is_none_or(|filter| self.passes_filter(&contact.inner, filter))
    }

    /// Whether [`Self::insert_filtered`] keeps `peer_contact` under `filter`: it provides the
    /// services we need (or both we and it are validators), advertises a secure websocket address
    /// if we require one, and is neither from the future nor older than [`Self::MAX_PEER_AGE`].
    fn passes_filter(&self, peer_contact: &PeerContact, filter: &InsertFilter) -> bool {
        // A peer is interesting to us in two cases:
        // - We are configured as a validator, and the peer is also a validator, then that peer is
        //   interesting regardless of the services that are provided by that peer.
        // - The services provided by the peer are a superset of the requested services.
        let we_are_validator = self
            .own_peer_contact
            .services()
            .contains(Services::VALIDATOR);
        let keep_validator =
            we_are_validator && peer_contact.services.contains(Services::VALIDATOR);
        if !keep_validator && !peer_contact.services.contains(filter.services) {
            return false;
        }

        // Check that the peer provides secure ws addresses if required.
        if filter.only_secure_ws_connections
            && !peer_contact
                .addresses
                .iter()
                .any(utils::is_address_ws_secure)
        {
            return false;
        }

        let current_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Reject contacts with timestamps in the future, and contacts that are older than the
        // allowed age.
        peer_contact.timestamp <= current_ts
            && !contact_exceeds_age(
                peer_contact.timestamp,
                Duration::from_secs(PeerContactBook::MAX_PEER_AGE),
                Duration::from_secs(current_ts),
            )
    }

    /// Insert a peer contact or update an existing one
    pub fn insert(&mut self, contact: impl Into<CheckedPeerContact>) {
        let contact = contact.into();

        // Don't insert our own contact into our peer contacts
        if contact.signed().peer_id() == self.own_peer_id {
            return;
        }

        log::debug!(peer_id = %contact.signed().peer_id(), addresses = ?contact.signed().inner.addresses, "Adding peer contact");
        let current_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Reject contacts with timestamps in the future
        if contact.signed().inner.timestamp > current_ts {
            return;
        }

        self.store(contact);
    }

    /// Stores a contact, replacing an existing one only if the new one is strictly newer.
    ///
    /// Keeps the latest contact separate from the peer's verified validator binding, so pending
    /// refreshes cannot extend the lifetime or priority of a previously verified binding.
    ///
    /// The one exception to "strictly newer" is an identical copy of the stored contact whose
    /// claim is still pending: if the copy's claim was conclusively checked, that outcome settles
    /// the stored claim just like [`Self::apply_validator_verifications`] would. Copies of the same
    /// contact can be checked on several connections before the first one is stored, and only one
    /// of them may get to spend the budget on it (see
    /// [`CheckedPeerContact::check_new_with_budget`]). Without this, a copy left pending that
    /// happened to be stored first would shadow the verified one until the re-check sweep got to
    /// it. A claim that was already conclusively checked is left as it is.
    fn store(&mut self, checked: CheckedPeerContact) {
        let peer_id = checked.contact.inner.peer_id();
        let existing = self.peer_contacts.get(&peer_id).cloned();

        if let Some(existing) = &existing {
            // Only update the contact if the timestamp is greater than the entry we have
            if existing.contact().timestamp >= checked.contact.inner.timestamp {
                if existing.validator_claim_needs_recheck() && existing.signed() == &checked.contact
                {
                    self.settle_pending_claim(existing, checked.claim);
                }
                return;
            }
        } else {
            log::trace!(
                peer_id = %peer_id,
                services = ?checked.contact.inner.services,
                addresses = ?checked.contact.inner.addresses,
                validator_address = ?checked.contact.inner.validator_address(),
                "Adding peer contact",
            );
        }

        // Only a conclusive check can make the new contact trusted and gossipable. A pending
        // refresh with the same address keeps the previous verified binding until it expires.
        let (validator_verified, pending) = match checked.claim {
            ClaimCheck::Verified => (true, false),
            ClaimCheck::Invalid | ClaimCheck::None => (false, false),
            ClaimCheck::Pending => (false, true),
        };

        let info = Arc::new(PeerContactInfo::new(
            checked.contact,
            validator_verified,
            pending,
        ));

        self.peer_contacts.insert(peer_id, Arc::clone(&info));

        // Keep the verified binding across pending refreshes only while the address stays the
        // same. A conclusive rejection, a changed address or a dropped claim removes it.
        if let Some(replaced) = existing
            && (!pending || replaced.validator_address() != info.validator_address())
        {
            self.index_remove(&replaced);
        }
        self.index_add(&info);
    }

    /// Settles the pending claim of the stored contact `info` with the outcome `claim` of checking
    /// an identical copy of it. See [`Self::store`].
    fn settle_pending_claim(&mut self, info: &PeerContactInfo, claim: ClaimCheck) {
        match claim {
            ClaimCheck::Verified => {
                info.set_validator_verified(true);
                self.index_add(info);
            }
            ClaimCheck::Invalid => {
                info.set_validator_verified(false);
                self.index_remove(info);
            }
            // The copy was not conclusively checked either (or, converted from a plain contact,
            // not checked at all), so there is nothing to settle.
            ClaimCheck::Pending | ClaimCheck::None => {}
        }
    }

    /// Records a verified binding at this contact's timestamp, only if this exact contact was
    /// verified.
    ///
    /// A validator address keeps at most [`Self::MAX_BINDINGS_PER_VALIDATOR`] live bindings, the
    /// newest ones, ranked like [`Self::get_validator_peer_ids`] ranks them. A binding that does not
    /// make it, the new one included, is dropped, and the contact it was verified in is no longer
    /// considered verified, so that it is neither reported nor gossiped. Only the validator's
    /// signing key can produce bindings to its address, so this only ever limits the validator
    /// itself: an honest one binds one peer, or two for a while after it moved.
    fn index_add(&mut self, info: &PeerContactInfo) {
        if !info.is_validator_verified() {
            return;
        }
        let Some(validator_address) = info.validator_address() else {
            return;
        };

        let bindings = self
            .verified_claim_timestamps
            .entry(validator_address.clone())
            .or_default();
        bindings.insert(info.peer_id, info.contact().timestamp());
        let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
            return;
        };

        // Expired bindings are no longer reported, so they do not count towards the limit. They
        // are left for house-keeping to remove.
        let mut live: Vec<(PeerId, u64)> = bindings
            .iter()
            .filter(|(_, timestamp)| {
                !contact_exceeds_age(
                    **timestamp,
                    Duration::from_secs(Self::MAX_PEER_AGE),
                    unix_time,
                )
            })
            .map(|(&peer_id, &timestamp)| (peer_id, timestamp))
            .collect();
        let mut evicted = Vec::new();
        if live.len() > Self::MAX_BINDINGS_PER_VALIDATOR {
            // Rank like `get_validator_peer_ids`: newest first, then by peer ID.
            live.sort_unstable_by(|(peer_a, ts_a), (peer_b, ts_b)| {
                ts_b.cmp(ts_a).then_with(|| peer_a.cmp(peer_b))
            });
            for (peer_id, _) in live.drain(Self::MAX_BINDINGS_PER_VALIDATOR..) {
                bindings.remove(&peer_id);
                evicted.push(peer_id);
            }
        }

        for peer_id in evicted {
            debug!(
                %peer_id,
                %validator_address,
                "Dropping validator binding, the validator has newer ones",
            );
            if let Some(evicted_info) = self.peer_contacts.get(&peer_id)
                && evicted_info.is_validator_verified()
                && evicted_info.validator_address() == Some(validator_address)
            {
                evicted_info.set_validator_verified(false);
            }
        }
    }

    /// Removes the verified binding between this contact's peer and its validator address.
    /// Returns whether there was one.
    fn index_remove(&mut self, info: &PeerContactInfo) -> bool {
        let Some(validator_address) = info.validator_address() else {
            return false;
        };
        let Entry::Occupied(mut entry) = self
            .verified_claim_timestamps
            .entry(validator_address.clone())
        else {
            return false;
        };
        let removed = entry.get_mut().remove(&info.peer_id).is_some();
        if entry.get().is_empty() {
            entry.remove();
        }
        removed
    }

    /// Inserts a peer contact or update an existing using the service filtering.
    /// If the filter matches the services provided by the contact, it is added.
    /// Otherwise it is ignored.
    /// The services_filter argument to this function contains the services that are required.
    pub fn insert_filtered(
        &mut self,
        contact: impl Into<CheckedPeerContact>,
        services_filter: Services,
        only_secure_ws_connections: bool,
    ) {
        let contact = contact.into();

        // Don't insert our own contact into our peer contacts. Peers do echo it back to us.
        if contact.signed().peer_id() == self.own_peer_id {
            return;
        }

        let filter = InsertFilter {
            services: services_filter,
            only_secure_ws_connections,
        };
        if !self.passes_filter(&contact.signed().inner, &filter) {
            return;
        }

        self.store(contact);
    }

    /// Inserts a set of contacts or updates existing ones
    pub fn insert_all<C: Into<CheckedPeerContact>, I: IntoIterator<Item = C>>(
        &mut self,
        contacts: I,
    ) {
        for contact in contacts {
            self.insert(contact);
        }
    }

    /// Inserts a set of peer contact or update an existing ones using the service
    /// filtering. If the filter matches the services provided by the contact,
    /// it is added. Otherwise it is ignored.
    pub fn insert_all_filtered<C: Into<CheckedPeerContact>, I: IntoIterator<Item = C>>(
        &mut self,
        contacts: I,
        services_filter: Services,
        only_secure_ws_connections: bool,
    ) {
        for contact in contacts {
            self.insert_filtered(contact, services_filter, only_secure_ws_connections)
        }
    }

    /// Gets a peer contact if it exists given its peer_id.
    /// If the peer_id is not found, `None` is returned.
    pub fn get(&self, peer_id: &PeerId) -> Option<Arc<PeerContactInfo>> {
        self.peer_contacts.get(peer_id).cloned()
    }

    /// Gets the peer contact's addresses if it exists given its peer_id.
    /// If the peer_id is not found, `None` is returned.
    pub fn get_addresses(&self, peer_id: &PeerId) -> Option<Vec<Multiaddr>> {
        self.peer_contacts.get(peer_id).map(|e| {
            let peer_contact = e.contact();
            peer_contact
                .addresses
                .iter()
                .filter(|&address| self.is_address_dialable(address))
                .cloned()
                .collect()
        })
    }

    /// The peer IDs known to belong to `validator_address`, newest verified binding first.
    ///
    /// A binding is ranked and aged by the timestamp of the contact whose claim was verified, and
    /// a pending refresh changes neither. Expiry is checked on every lookup, independently of
    /// house-keeping, and the peer's latest contact must still claim this validator address. A
    /// newer contact whose claim to this address is conclusively rejected also ends the binding,
    /// whether the claim is rejected on insertion or by the re-check sweep (see
    /// [`Self::apply_validator_verifications`]).
    pub fn get_validator_peer_ids(&self, validator_address: &Address) -> Vec<PeerId> {
        let Some(verified) = self.verified_claim_timestamps.get(validator_address) else {
            return Vec::new();
        };
        let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
            return Vec::new();
        };

        let mut bindings: Vec<(PeerId, u64)> = verified
            .iter()
            .map(|(&peer_id, &timestamp)| (peer_id, timestamp))
            .filter(|(peer_id, timestamp)| {
                !contact_exceeds_age(
                    *timestamp,
                    Duration::from_secs(Self::MAX_PEER_AGE),
                    unix_time,
                ) && self
                    .peer_contacts
                    .get(peer_id)
                    .is_some_and(|info| info.validator_address() == Some(validator_address))
            })
            .collect();

        // Prefer the most recent claim: a validator that moved to another node should win over the
        // contact of the node it left behind.
        bindings.sort_unstable_by(|(peer_a, ts_a), (peer_b, ts_b)| {
            ts_b.cmp(ts_a).then_with(|| peer_a.cmp(peer_b))
        });

        bindings.into_iter().map(|(peer_id, _)| peer_id).collect()
    }

    /// Whether `peer_id` holds a verified binding to `validator_address` that has not expired yet.
    ///
    /// Such a binding only ever results from a conclusive check of a claim signed with the
    /// validator's signing key, so this is what entitles a refreshed contact to the refresh
    /// reserve of the claim budget (see [`CheckedPeerContact::check_new_with_budget`]). Expiry is
    /// checked the same way as in [`Self::get_validator_peer_ids`], independently of
    /// house-keeping.
    pub fn has_live_verified_binding(&self, peer_id: &PeerId, validator_address: &Address) -> bool {
        let Some(&timestamp) = self
            .verified_claim_timestamps
            .get(validator_address)
            .and_then(|bindings| bindings.get(peer_id))
        else {
            return false;
        };
        let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
            return false;
        };
        !contact_exceeds_age(
            timestamp,
            Duration::from_secs(Self::MAX_PEER_AGE),
            unix_time,
        )
    }

    /// Snapshot of the contacts whose validator claim still needs a fresh cryptographic check —
    /// it has not been conclusively checked yet (skipped for budget or came back
    /// [`ValidatorVerification::Unverifiable`]). Claims that were conclusively checked —
    /// `Verified` or `Invalid` — are not included, so a peer cannot keep the sweep busy by
    /// attaching claims that are rejected outright.
    ///
    /// The result is ordered by peer ID rather than left in arbitrary hash-map order, so that a
    /// caller re-checking only a bounded prefix each tick (see
    /// [`super::behaviour::Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK`]) can rotate which prefix it
    /// takes and still give every contact a bounded wait, instead of always favoring whichever
    /// contacts happen to land first in `HashMap` iteration order.
    ///
    /// Verification needs the staking contract, so it must happen outside the contact book lock.
    /// Take this snapshot, check the claims, then feed the results back through
    /// [`PeerContactBook::apply_validator_verifications`].
    pub fn unverified_validator_contacts(&self) -> Vec<Arc<PeerContactInfo>> {
        let mut contacts: Vec<Arc<PeerContactInfo>> = self
            .peer_contacts
            .values()
            .filter(|info| {
                info.validator_address().is_some() && info.validator_claim_needs_recheck()
            })
            .cloned()
            .collect();
        contacts.sort_unstable_by_key(|info| info.peer_id);
        contacts
    }

    /// Snapshot of the contacts of peers whose live verified binding went stale: the binding is
    /// older than [`Self::STALE_BINDING_AGE`], and the peer's current contact is either the one it
    /// was verified in or a pending refresh claiming the same validator. Ordered by peer ID, like
    /// [`Self::unverified_validator_contacts`]. A binding that was picked for a re-check less than
    /// [`Self::STALE_BINDING_AGE`] ago is left out (see [`PeerContactInfo::mark_binding_rechecked`]).
    ///
    /// A claim is checked against the staking contract when its contact arrives, and a validator
    /// that is online refreshes its contact, and thereby the check, on every house-keeping tick.
    /// A binding that stopped being refreshed would otherwise be trusted until it expires, even if
    /// the validator rotated its signing key in the meantime, and so would one whose refreshes are
    /// left pending, e.g. because they are signed with a key rotated away and the claim budget is
    /// used up. The re-check sweep re-checks the current contact: a conclusive rejection removes
    /// the binding, and a verification of a pending refresh renews it (see
    /// [`Self::apply_validator_verifications`]).
    pub fn stale_verified_validator_contacts(&self) -> Vec<Arc<PeerContactInfo>> {
        let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
            return Vec::new();
        };
        let mut contacts: Vec<Arc<PeerContactInfo>> = self
            .peer_contacts
            .values()
            .filter(|info| {
                let Some(&bound_at) = info.validator_address().and_then(|address| {
                    self.verified_claim_timestamps
                        .get(address)
                        .and_then(|bindings| bindings.get(&info.peer_id))
                }) else {
                    return false;
                };
                let checkable = (info.is_validator_verified()
                    && bound_at == info.contact().timestamp())
                    || info.validator_claim_needs_recheck();
                checkable
                    && contact_exceeds_age(
                        bound_at,
                        Duration::from_secs(Self::STALE_BINDING_AGE),
                        unix_time,
                    )
                    && !contact_exceeds_age(
                        bound_at,
                        Duration::from_secs(Self::MAX_PEER_AGE),
                        unix_time,
                    )
                    && !info.binding_rechecked_recently(unix_time)
            })
            .cloned()
            .collect();
        contacts.sort_unstable_by_key(|info| info.peer_id);
        contacts
    }

    /// Applies validator claim checks computed outside the lock.
    ///
    /// Each result carries the contact timestamp it was computed from; results for a contact
    /// that has since been replaced are discarded. A conclusive verification establishes or
    /// refreshes the peer's verified binding at the checked contact's timestamp, but only while
    /// the contact's claim is still pending: if the claim was settled in the meantime, e.g. by an
    /// identical copy checked on arrival (see [`Self::store`]), that outcome stands, as the check
    /// here may have read older state. A conclusive rejection always removes the binding, even
    /// when the current contact is already unverified or was verified before, so that a claim
    /// that no longer holds, e.g. after the validator rotated its signing key, cannot outlive the
    /// check that found out (see [`Self::stale_verified_validator_contacts`]). An inconclusive
    /// result leaves the binding's original expiry intact.
    pub fn apply_validator_verifications(
        &mut self,
        results: impl IntoIterator<Item = (PeerId, u64, ValidatorVerification)>,
    ) {
        for (peer_id, timestamp, verification) in results {
            let Some(info) = self.peer_contacts.get(&peer_id) else {
                continue;
            };
            if info.contact().timestamp != timestamp {
                continue;
            }

            match verification {
                ValidatorVerification::Verified if !info.validator_claim_needs_recheck() => {
                    trace!(%peer_id, "Ignoring verification of a claim that was settled since");
                }
                ValidatorVerification::Verified => {
                    debug!(%peer_id, validator_address = ?info.validator_address(), "Verified validator claim of peer contact");
                    info.set_validator_verified(true);
                    let info = Arc::clone(info);
                    self.index_add(&info);
                }
                ValidatorVerification::Invalid(reason) => {
                    info.set_validator_verified(false);
                    let info = Arc::clone(info);
                    let removed_binding = self.index_remove(&info);
                    debug!(
                        %peer_id,
                        ?reason,
                        removed_binding,
                        "Validator claim of peer contact failed a definitive re-check",
                    );
                }
                // Inconclusive: the current contact stays pending and the verified binding, if
                // any, keeps its original timestamp and expiry.
                ValidatorVerification::Unverifiable(_) => {}
            }
        }
    }

    /// Retrieves a single PeerInfo object for every known peer.
    /// Additional addresses aside from the first are omitted.
    ///
    /// The returned Vec may be empty.
    pub fn known_peers(&self) -> Vec<(PeerId, PeerInfo)> {
        self.peer_contacts
            .iter()
            .filter_map(|(peer_id, contact)| {
                let address = contact.contact.inner.addresses.first()?.clone();
                Some((
                    *peer_id,
                    PeerInfo::new(address, contact.contact.inner.services),
                ))
            })
            .collect()
    }

    /// Gets a set of peer contacts given a services filter.
    /// Every peer contact that matches such services will be returned.
    pub fn query(&self, services: Services) -> impl Iterator<Item = Arc<PeerContactInfo>> + '_ {
        // TODO: This is a naive implementation
        // TODO: Sort by score?
        self.peer_contacts.values().filter_map(move |contact| {
            if contact.matches(services) {
                Some(Arc::clone(contact))
            } else {
                None
            }
        })
    }

    /// Updates the score of every peer in the contact book with the gossipsub
    /// peer score.
    pub fn update_scores(&self, gossipsub: &gossipsub::Behaviour) {
        let contacts = self.peer_contacts.iter();

        for contact in contacts {
            if let Some(score) = gossipsub.peer_score(contact.0) {
                contact.1.set_score(score);
            } else {
                debug!(peer_id = %contact.0, "No score for peer");
            }
        }
    }

    /// Adds a set of addresses to the list of addresses known for our own contact.
    pub fn add_own_addresses<I: IntoIterator<Item = Multiaddr>>(
        &mut self,
        addresses: I,
        keypair: &Keypair,
    ) {
        let mut contact = self.own_peer_contact.contact.inner.clone();
        let addresses = addresses.into_iter().collect::<Vec<Multiaddr>>();
        trace!(?addresses, "Adding own addresses");
        contact.add_addresses(addresses);
        self.sign_own_contact(contact, keypair);
    }

    /// Removes a set of addresses from the list of addresses known for our own.
    pub fn remove_own_addresses<I: IntoIterator<Item = Multiaddr>>(
        &mut self,
        addresses: I,
        keypair: &Keypair,
    ) {
        let mut contact = self.own_peer_contact.contact.inner.clone();
        let addresses = addresses.into_iter().collect::<Vec<Multiaddr>>();
        contact.remove_addresses(addresses);
        self.sign_own_contact(contact, keypair);
    }

    /// Updates the timestamp of our own contact
    pub fn update_own_contact(&mut self, keypair: &Keypair) {
        // Not really optimal to clone here, but *shrugs*
        let mut contact = self.own_peer_contact.contact.inner.clone();

        // Update timestamp
        contact.set_current_time();

        self.sign_own_contact(contact, keypair);
    }

    /// Installs or removes the signer for the validator claim on our own contact.
    ///
    /// Our own contact is re-signed immediately so that the change takes effect without waiting
    /// for the next house-keeping tick.
    pub fn set_validator_claim_signer(
        &mut self,
        signer: Option<ValidatorClaimSigner>,
        keypair: &Keypair,
    ) {
        self.validator_claim_signer = signer;
        self.update_own_contact(keypair);
    }

    /// Signs `contact` as our own contact, attaching a fresh validator claim if we have a signer.
    ///
    /// The claim covers the contact's timestamp, so it has to be produced here, every time the
    /// contact is (re-)signed, rather than being handed to us pre-computed.
    fn sign_own_contact(&mut self, mut contact: PeerContact, keypair: &Keypair) {
        let validator_info = self.validator_claim_signer.as_ref().map(|signer| {
            let signed_claim = signer.sign(contact.peer_id(), contact.timestamp());
            ValidatorInfo::new(signer.validator_address().clone(), signed_claim.signature)
        });
        contact.set_validator_info(validator_info);

        self.own_peer_contact = PeerContactInfo::from(contact.sign(keypair));
    }

    /// Gets our own contact information
    pub fn get_own_contact(&self) -> &PeerContactInfo {
        &self.own_peer_contact
    }

    /// Removes peer contacts that have already exceeded the maximum age as
    /// defined in `MAX_PEER_AGE`, and verified validator bindings whose verified contact has.
    pub fn house_keeping(&mut self) {
        if let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            let delete_peers = self
                .peer_contacts
                .iter()
                .filter_map(|(peer_id, peer_contact)| {
                    if peer_contact.exceeds_age(
                        Duration::from_secs(PeerContactBook::MAX_PEER_AGE),
                        unix_time,
                    ) {
                        debug!(%peer_id, "Removing peer contact because of old age");
                        Some(peer_id)
                    } else {
                        None
                    }
                })
                .cloned()
                .collect::<Vec<PeerId>>();

            for peer_id in delete_peers {
                if let Some(info) = self.peer_contacts.remove(&peer_id) {
                    self.index_remove(&info);
                }
            }

            // A fresh pending contact must not keep an older verified binding alive.
            self.verified_claim_timestamps.retain(|_, bindings| {
                bindings.retain(|_, timestamp| {
                    !contact_exceeds_age(
                        *timestamp,
                        Duration::from_secs(Self::MAX_PEER_AGE),
                        unix_time,
                    )
                });
                !bindings.is_empty()
            });
        }
    }

    /// Returns true if an address is valid for dialing.
    /// It performs basic checks against unsupported addresses.
    pub fn is_address_dialable(&self, address: &Multiaddr) -> bool {
        // If we use a memory transport, we don't do any check
        if self.memory_transport {
            return true;
        }
        // Otherwise check for an appropriate WS address
        let mut protocols = address.iter();
        let mut ip = protocols.next();
        let mut tcp = protocols.next();
        // The encapsulating protocol must be based on TCP/IP, possibly via DNS.
        let is_dns = loop {
            match (ip, tcp) {
                (Some(Protocol::Ip4(ip)), Some(Protocol::Tcp(_))) => {
                    if !self.allow_loopback_addresses && ip.is_loopback() {
                        return false;
                    }
                    break false;
                }
                (Some(Protocol::Ip6(ip)), Some(Protocol::Tcp(_))) => {
                    if !self.allow_loopback_addresses && ip.is_loopback() {
                        return false;
                    }
                    break false;
                }
                (Some(Protocol::Dns(_)), Some(Protocol::Tcp(_)))
                | (Some(Protocol::Dns4(_)), Some(Protocol::Tcp(_)))
                | (Some(Protocol::Dns6(_)), Some(Protocol::Tcp(_)))
                | (Some(Protocol::Dnsaddr(_)), Some(Protocol::Tcp(_))) => break true,
                (Some(_), Some(p)) => {
                    ip = Some(p);
                    tcp = protocols.next();
                }
                _ => return false,
            }
        };

        // Now check the `Ws` / `Wss` protocol from the end of the address,
        // that could also have a trailing `P2p` protocol that identifies the remote.
        let mut protocols: Multiaddr = address.clone();
        loop {
            match protocols.pop() {
                Some(Protocol::P2p(_)) => {}
                Some(Protocol::Ws(_)) => return !self.only_secure_addresses,
                Some(Protocol::Wss(_)) => {
                    if !is_dns {
                        trace!(address=%address, "Missing DNS name in WSS address");
                        return false;
                    }
                    return true;
                }
                _ => return false,
            }
        }
    }
}

mod serde_public_key {
    use libp2p::identity::PublicKey;
    use serde::{
        de::Error, ser::Error as SerializationError, Deserialize, Deserializer, Serialize,
        Serializer,
    };

    pub fn serialize<S>(public_key: &PublicKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Ok(pk) = public_key.clone().try_into_ed25519() {
            Serialize::serialize(&pk.to_bytes(), serializer)
        } else {
            Err(S::Error::custom("Unsupported key type"))
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PublicKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_encoded: [u8; 32] = Deserialize::deserialize(deserializer)?;

        let pk = libp2p::identity::ed25519::PublicKey::try_from_bytes(&hex_encoded)
            .map_err(|_| D::Error::custom("Invalid value"))?;

        Ok(PublicKey::from(pk))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nimiq_keys::{Ed25519PublicKey, SecureGenerate};
    use nimiq_network_interface::validator_record::ValidatorRecord;
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;

    use super::*;
    use crate::discovery::validator_verifier::{
        InvalidReason, NoopValidatorClaimVerifier, UnverifiableReason,
    };

    /// A verifier that knows a fixed set of validator signing keys.
    struct TestVerifier {
        keys: HashMap<Address, Ed25519PublicKey>,
    }

    impl ValidatorClaimVerifier for TestVerifier {
        fn verify_validator_claim(
            &self,
            signed_claim: &SignedValidatorClaim,
        ) -> ValidatorVerification {
            let Some(public_key) = self.keys.get(&signed_claim.record.validator_address) else {
                return ValidatorVerification::Invalid(InvalidReason::UnknownValidator);
            };
            if signed_claim.verify(public_key) {
                ValidatorVerification::Verified
            } else {
                ValidatorVerification::Invalid(InvalidReason::InvalidSignature)
            }
        }
    }

    /// A verifier that answers every claim with the same outcome and counts how often it was
    /// asked, i.e. how many blockchain reads a real verifier would have done.
    struct CountingVerifier {
        outcome: ValidatorVerification,
        calls: AtomicUsize,
    }

    impl CountingVerifier {
        fn new(outcome: ValidatorVerification) -> Self {
            Self {
                outcome,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl ValidatorClaimVerifier for CountingVerifier {
        fn verify_validator_claim(
            &self,
            _signed_claim: &SignedValidatorClaim,
        ) -> ValidatorVerification {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.outcome
        }
    }

    /// A verifier that fails the test if it is asked to check a claim while `book` is locked,
    /// and otherwise defers to `inner`.
    struct LockCheckingVerifier<'a> {
        book: &'a RwLock<PeerContactBook>,
        inner: &'a dyn ValidatorClaimVerifier,
    }

    impl ValidatorClaimVerifier for LockCheckingVerifier<'_> {
        fn verify_validator_claim(
            &self,
            signed_claim: &SignedValidatorClaim,
        ) -> ValidatorVerification {
            assert!(
                self.book.try_write().is_some(),
                "a claim must never be checked while the contact book is locked",
            );
            self.inner.verify_validator_claim(signed_claim)
        }
    }

    /// A book holding a live verified binding of a peer to the validator of `signer`, with the key
    /// of that peer.
    fn book_with_binding(
        signer: &ValidatorClaimSigner,
        verifier: &TestVerifier,
    ) -> (Keypair, RwLock<PeerContactBook>) {
        let (_own_key, mut book) = empty_book();
        let peer_key = Keypair::generate_ed25519();
        let first = contact_for(&peer_key, now_secs() - 10, Some(signer));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check(first, verifier));
        assert!(book.has_live_verified_binding(&peer_id, signer.validator_address()));
        (peer_key, RwLock::new(book))
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// Creates a validator signing key and a verifier that accepts it.
    fn validator(seed: u8) -> (ValidatorClaimSigner, TestVerifier) {
        let key_pair = KeyPair::generate(&mut test_rng(false));
        let mut address_bytes = [0u8; 20];
        address_bytes[0] = seed;
        let address = Address::from(address_bytes);

        let mut keys = HashMap::new();
        keys.insert(address.clone(), key_pair.public);

        (
            ValidatorClaimSigner::new(address, key_pair),
            TestVerifier { keys },
        )
    }

    fn contact_for(
        keypair: &Keypair,
        timestamp: u64,
        signer: Option<&ValidatorClaimSigner>,
    ) -> SignedPeerContact {
        let mut contact = PeerContact::new(
            ["/ip4/127.0.0.1/tcp/8443".parse().unwrap()],
            keypair.public(),
            Services::all(),
            timestamp,
        )
        .unwrap();

        if let Some(signer) = signer {
            let signed_claim = signer.sign(contact.peer_id(), timestamp);
            contact.set_validator_info(Some(ValidatorInfo::new(
                signer.validator_address().clone(),
                signed_claim.signature,
            )));
        }

        contact.sign(keypair)
    }

    fn empty_book() -> (Keypair, PeerContactBook) {
        let keypair = Keypair::generate_ed25519();
        let own_contact = contact_for(&keypair, now_secs(), None);
        (
            keypair.clone(),
            PeerContactBook::new(own_contact, false, true, true),
        )
    }

    #[test]
    fn verified_claim_is_indexed() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let contact = contact_for(&peer_key, now_secs(), Some(&signer));
        let peer_id = contact.peer_id();

        book.insert(CheckedPeerContact::check(contact, &verifier));

        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert!(book.get(&peer_id).unwrap().is_gossipable());
    }

    #[test]
    fn unverified_claim_is_stored_but_neither_indexed_nor_gossiped() {
        let (_own_key, mut book) = empty_book();
        let (signer, _verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let contact = contact_for(&peer_key, now_secs(), Some(&signer));
        let peer_id = contact.peer_id();

        book.insert(CheckedPeerContact::check(
            contact,
            &NoopValidatorClaimVerifier,
        ));

        // The contact is kept and remains dialable, we just do not vouch for its claim.
        let info = book.get(&peer_id).expect("contact must be stored");
        assert!(!info.is_validator_verified());
        assert!(!info.is_gossipable());
        assert!(!book.get_addresses(&peer_id).unwrap().is_empty());
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
    }

    #[test]
    fn replacing_a_contact_moves_the_index_to_the_new_address() {
        let (_own_key, mut book) = empty_book();
        let (signer_a, verifier_a) = validator(1);
        let (signer_b, verifier_b) = validator(2);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let first = contact_for(&peer_key, now - 10, Some(&signer_a));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check(first, &verifier_a));
        assert_eq!(
            book.get_validator_peer_ids(signer_a.validator_address()),
            vec![peer_id]
        );

        // The same peer now claims a different validator.
        let second = contact_for(&peer_key, now, Some(&signer_b));
        book.insert(CheckedPeerContact::check(second, &verifier_b));

        assert!(book
            .get_validator_peer_ids(signer_a.validator_address())
            .is_empty());
        assert_eq!(
            book.get_validator_peer_ids(signer_b.validator_address()),
            vec![peer_id]
        );
    }

    #[test]
    fn dropping_the_claim_drops_the_index_entry() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let first = contact_for(&peer_key, now - 10, Some(&signer));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check(first, &verifier));

        // A pending refresh with the same address keeps the verified binding.
        let pending = contact_for(&peer_key, now - 5, Some(&signer));
        book.insert(CheckedPeerContact::check(
            pending,
            &NoopValidatorClaimVerifier,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert!(!book.get(&peer_id).unwrap().is_validator_verified());

        // A newer contact from the same peer without any claim.
        let second = contact_for(&peer_key, now, None);
        book.insert(CheckedPeerContact::check(second, &verifier));

        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
        // The lookup also filters on the current claim, so check the binding itself is gone.
        assert!(book.verified_claim_timestamps.is_empty());
    }

    #[test]
    fn stale_contact_does_not_replace_a_newer_one() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let newer = contact_for(&peer_key, now, None);
        book.insert(CheckedPeerContact::check(newer, &verifier));

        // An older contact carrying a claim must not be able to re-add the peer to the index.
        let older = contact_for(&peer_key, now - 10, Some(&signer));
        book.insert(CheckedPeerContact::check(older, &verifier));

        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
    }

    #[test]
    fn house_keeping_removes_index_entries() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let stale_ts = now_secs() - PeerContactBook::MAX_PEER_AGE * 2;

        let contact = contact_for(&peer_key, stale_ts, Some(&signer));
        book.insert(CheckedPeerContact::check(contact, &verifier));
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()].len(),
            1
        );

        book.house_keeping();

        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
        assert!(!book
            .verified_claim_timestamps
            .contains_key(signer.validator_address()));
    }

    #[test]
    fn verifications_apply_only_to_the_contact_they_were_computed_from() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let contact = contact_for(&peer_key, now, Some(&signer));
        let peer_id = contact.peer_id();
        book.insert(CheckedPeerContact::check(
            contact,
            &NoopValidatorClaimVerifier,
        ));

        // A result computed from a different (older) version of the contact is discarded.
        book.apply_validator_verifications([(peer_id, now - 1, ValidatorVerification::Verified)]);
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());

        // Re-checking the contact we actually hold promotes it.
        let snapshot = book.unverified_validator_contacts();
        assert_eq!(snapshot.len(), 1);
        let results: Vec<_> = snapshot
            .iter()
            .map(|info| {
                (
                    *info.peer_id(),
                    info.contact().timestamp(),
                    info.signed().check_validator_claim(&verifier).unwrap(),
                )
            })
            .collect();
        book.apply_validator_verifications(results);

        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert!(book.unverified_validator_contacts().is_empty());
    }

    #[test]
    fn newest_claim_is_returned_first() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let now = now_secs();

        let old_key = Keypair::generate_ed25519();
        let old = contact_for(&old_key, now - 60, Some(&signer));
        let old_peer_id = old.peer_id();
        book.insert(CheckedPeerContact::check(old, &verifier));

        let new_key = Keypair::generate_ed25519();
        let new = contact_for(&new_key, now, Some(&signer));
        let new_peer_id = new.peer_id();
        book.insert(CheckedPeerContact::check(new, &verifier));

        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![new_peer_id, old_peer_id]
        );
    }

    #[test]
    fn insert_filtered_ignores_our_own_contact() {
        let (_own_key, mut book) = empty_book();
        let own_contact = book.get_own_contact().signed().clone();
        let own_peer_id = own_contact.peer_id();

        book.insert_filtered(own_contact, Services::all(), false);

        assert!(book.get(&own_peer_id).is_none());
    }

    #[test]
    fn own_contact_carries_a_verifiable_validator_claim() {
        let (own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let validator_address = signer.validator_address().clone();

        book.set_validator_claim_signer(Some(signer), &own_key);

        let own_contact = book.get_own_contact().signed().clone();
        assert!(own_contact.verify());
        assert_eq!(
            own_contact.inner.validator_address(),
            Some(&validator_address)
        );
        assert_eq!(
            own_contact.check_validator_claim(&verifier),
            Some(ValidatorVerification::Verified)
        );

        // Refreshing the contact re-signs the claim for the new timestamp.
        book.update_own_contact(&own_key);
        let refreshed = book.get_own_contact().signed().clone();
        assert_eq!(
            refreshed.check_validator_claim(&verifier),
            Some(ValidatorVerification::Verified)
        );

        book.set_validator_claim_signer(None, &own_key);
        assert!(book
            .get_own_contact()
            .signed()
            .inner
            .validator_info()
            .is_none());
    }

    #[test]
    fn claims_and_dht_records_are_not_interchangeable() {
        let key_pair = KeyPair::generate(&mut test_rng(false));
        let address = Address::from([1u8; 20]);
        let peer_id = Keypair::generate_ed25519().public().to_peer_id();
        let timestamp = now_secs();

        // A claim gossiped in a peer contact does not pass as a DHT record...
        let claim =
            ValidatorClaimSigner::new(address.clone(), key_pair.clone()).sign(peer_id, timestamp);
        assert!(claim.verify(&key_pair.public));
        let claim_as_record = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::new(
            ValidatorRecord::new(peer_id, address.clone(), timestamp),
            TaggedSignature::from_bytes(claim.signature.as_bytes().to_vec()),
        );
        assert!(!claim_as_record.verify(&key_pair.public));

        // ...and a DHT record does not pass as a claim.
        let record = ValidatorRecord::new(peer_id, address.clone(), timestamp);
        let record_signature = key_pair.tagged_sign(&record);
        let record_as_claim = TaggedSigned::<ValidatorClaim<PeerId>, KeyPair>::new(
            ValidatorClaim::new(peer_id, address, timestamp),
            TaggedSignature::from_bytes(record_signature.as_bytes().to_vec()),
        );
        assert!(!record_as_claim.verify(&key_pair.public));
    }

    #[test]
    fn check_with_budget_leaves_the_claim_unverified_once_exhausted() {
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let contact = contact_for(&peer_key, now_secs(), Some(&signer));

        let budget = ValidatorClaimBudget::new(0);
        let checked = CheckedPeerContact::check_with_budget(contact, &verifier, &budget);

        // The claim would verify, but no budget was available to check it.
        assert_eq!(checked.claim, ClaimCheck::Pending);
    }

    #[test]
    fn pending_refresh_keeps_original_verified_binding_until_promotion() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        // First sighting: budget is available, the claim verifies and gets indexed.
        let budget = ValidatorClaimBudget::new(1);
        let first = contact_for(&peer_key, now - 10, Some(&signer));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check_with_budget(
            first, &verifier, &budget,
        ));

        let other_key = Keypair::generate_ed25519();
        let other = contact_for(&other_key, now - 5, Some(&signer));
        let other_peer_id = other.peer_id();
        book.insert(CheckedPeerContact::check(other, &verifier));

        // The same peer refreshes its contact (newer timestamp, same claim), but this time the
        // shared budget is exhausted by contention from other connections.
        assert!(!budget.try_consume(), "budget should already be empty");
        for timestamp in [now - 2, now] {
            let refreshed = contact_for(&peer_key, timestamp, Some(&signer));
            book.insert(CheckedPeerContact::check_with_budget(
                refreshed.clone(),
                &verifier,
                &budget,
            ));

            // Repeated pending refreshes preserve only the original verified binding. They
            // neither improve its ordering nor make the latest contact gossipable.
            assert_eq!(
                book.get_validator_peer_ids(signer.validator_address()),
                vec![other_peer_id, peer_id]
            );
            assert_eq!(
                book.verified_claim_timestamps[signer.validator_address()][&peer_id],
                now - 10
            );
            let info = book.get(&peer_id).unwrap();
            assert_eq!(info.signed(), &refreshed);
            assert!(!info.is_validator_verified());
            assert!(!info.is_gossipable());
        }

        let pending = book.unverified_validator_contacts();
        assert_eq!(pending.len(), 1);
        let refreshed = pending[0].signed().clone();
        book.apply_validator_verifications([(
            peer_id,
            now,
            refreshed.check_validator_claim(&verifier).unwrap(),
        )]);

        // A conclusive success promotes the current contact and its timestamp together.
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id, other_peer_id]
        );
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()][&peer_id],
            now
        );
        assert!(book.get(&peer_id).unwrap().is_validator_verified());
        assert!(book.get(&peer_id).unwrap().is_gossipable());
        assert!(book.unverified_validator_contacts().is_empty());
    }

    #[test]
    fn budget_exhaustion_does_not_verify_a_brand_new_claim() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();

        // The very first sighting of this peer arrives while the budget is already exhausted.
        let budget = ValidatorClaimBudget::new(0);
        let contact = contact_for(&peer_key, now_secs(), Some(&signer));
        let peer_id = contact.peer_id();
        book.insert(CheckedPeerContact::check_with_budget(
            contact, &verifier, &budget,
        ));

        // Without a conclusive check, the claim stays unverified (safe default), not silently
        // trusted.
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
        assert!(!book.get(&peer_id).unwrap().is_validator_verified());
    }

    #[test]
    fn budget_exhaustion_does_not_carry_trust_over_to_a_new_validator_address() {
        let (_own_key, mut book) = empty_book();
        let (signer_a, verifier_a) = validator(1);
        let (signer_b, _verifier_b) = validator(2);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let budget = ValidatorClaimBudget::new(1);
        let first = contact_for(&peer_key, now - 10, Some(&signer_a));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check_with_budget(
            first,
            &verifier_a,
            &budget,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer_a.validator_address()),
            vec![peer_id]
        );

        // Establish a pending latest contact backed by the previous verified binding.
        let pending = contact_for(&peer_key, now - 5, Some(&signer_a));
        book.insert(CheckedPeerContact::check_with_budget(
            pending,
            &verifier_a,
            &budget,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer_a.validator_address()),
            vec![peer_id]
        );
        assert!(!book.get(&peer_id).unwrap().is_validator_verified());

        // The same peer now claims a *different* validator address while the budget is
        // exhausted. Trust in the old address must not transfer to the new one.
        assert!(!budget.try_consume());
        let switched = contact_for(&peer_key, now, Some(&signer_b));
        book.insert(CheckedPeerContact::check_with_budget(
            switched,
            &verifier_a,
            &budget,
        ));

        assert!(book
            .get_validator_peer_ids(signer_a.validator_address())
            .is_empty());
        assert!(book
            .get_validator_peer_ids(signer_b.validator_address())
            .is_empty());
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
        // The lookup also filters on the current claim, so check the binding itself is gone.
        assert!(book.verified_claim_timestamps.is_empty());
    }

    #[test]
    fn pending_address_flip_does_not_resurrect_the_old_binding() {
        let (_own_key, mut book) = empty_book();
        let (signer_a, verifier_a) = validator(1);
        let (signer_b, _verifier_b) = validator(2);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let first = contact_for(&peer_key, now - 20, Some(&signer_a));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check(first, &verifier_a));
        assert_eq!(
            book.get_validator_peer_ids(signer_a.validator_address()),
            vec![peer_id]
        );

        // The peer switches to another validator address and back, and neither claim gets a
        // conclusive check.
        let switched = contact_for(&peer_key, now - 10, Some(&signer_b));
        book.insert(CheckedPeerContact::check(
            switched,
            &NoopValidatorClaimVerifier,
        ));
        assert!(book.verified_claim_timestamps.is_empty());

        let back = contact_for(&peer_key, now, Some(&signer_a));
        book.insert(CheckedPeerContact::check(back, &NoopValidatorClaimVerifier));

        // The binding to the first address ended with the switch. Claiming that address again
        // without a conclusive check must not bring it back.
        assert!(book
            .get_validator_peer_ids(signer_a.validator_address())
            .is_empty());
        assert!(book.verified_claim_timestamps.is_empty());
        assert!(!book.get(&peer_id).unwrap().is_validator_verified());
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
    }

    #[test]
    fn check_with_budget_only_charges_contacts_that_carry_a_claim() {
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let with_claim = contact_for(&peer_key, now_secs(), Some(&signer));
        let without_claim = contact_for(&Keypair::generate_ed25519(), now_secs(), None);

        let budget = ValidatorClaimBudget::new(1);

        // A contact with no claim never touches the budget.
        let checked = CheckedPeerContact::check_with_budget(without_claim, &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::None);
        assert!(budget.try_consume());

        // Put the single unit back, then spend it on a contact that does carry a claim.
        budget.reset();
        let checked = CheckedPeerContact::check_with_budget(with_claim.clone(), &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Verified);

        // The budget is now exhausted, so a second claim in the same tick is left pending.
        let checked = CheckedPeerContact::check_with_budget(with_claim, &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Pending);
    }

    #[test]
    fn only_contacts_the_book_would_store_are_new() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();
        let contact = contact_for(&peer_key, now - 10, Some(&signer));

        assert!(book.is_new(&contact));
        book.insert(CheckedPeerContact::check(contact.clone(), &verifier));

        // The same contact, e.g. relayed to us by another peer, is not new, and neither is an older
        // one or our own.
        assert!(!book.is_new(&contact));
        assert!(!book.is_new(&contact_for(&peer_key, now - 20, Some(&signer))));
        assert!(!book.is_new(book.get_own_contact().signed()));
        assert!(!book.is_new(&contact_for(&peer_key, now + 60, Some(&signer))));

        // A newer contact is.
        assert!(book.is_new(&contact_for(&peer_key, now, Some(&signer))));
    }

    #[test]
    fn contacts_the_book_would_discard_do_not_use_up_the_budget() {
        let (own_key, mut book) = empty_book();
        let (own_signer, _) = validator(2);
        book.set_validator_claim_signer(Some(own_signer), &own_key);
        let own_contact = book.get_own_contact().signed().clone();
        let book = RwLock::new(book);

        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();
        let contact = contact_for(&peer_key, now - 10, Some(&signer));
        let budget = ValidatorClaimBudget::new(1);
        let check = |contact| {
            CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget).claim
        };

        assert_eq!(check(contact.clone()), ClaimCheck::Verified);
        book.write()
            .insert(CheckedPeerContact::check(contact.clone(), &verifier));

        // The same contact relayed to us again, and our own contact echoed back, are left pending
        // without being checked, so the single unit of budget is still there for a newer contact.
        budget.reset();
        assert_eq!(check(contact), ClaimCheck::Pending);
        assert_eq!(check(own_contact), ClaimCheck::Pending);
        assert_eq!(
            check(contact_for(&peer_key, now, Some(&signer))),
            ClaimCheck::Verified
        );
        assert!(!budget.try_consume());
    }

    #[test]
    fn a_contact_left_pending_is_offered_to_the_recheck_sweep() {
        let (_own_key, mut book) = empty_book();
        let (signer, _verifier) = validator(1);
        let with_claim = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
        let without_claim = contact_for(&Keypair::generate_ed25519(), now_secs(), None);

        assert_eq!(
            CheckedPeerContact::pending(without_claim).claim,
            ClaimCheck::None
        );
        book.insert(CheckedPeerContact::pending(with_claim));

        assert_eq!(book.unverified_validator_contacts().len(), 1);
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
    }

    #[test]
    fn invalid_refresh_removes_verified_binding_on_insert_or_recheck() {
        for check_on_insert in [false, true] {
            let (_own_key, mut book) = empty_book();
            let (signer, verifier) = validator(1);
            let peer_key = Keypair::generate_ed25519();
            let now = now_secs();

            let first = contact_for(&peer_key, now - 20, Some(&signer));
            let peer_id = first.peer_id();
            book.insert(CheckedPeerContact::check(first.clone(), &verifier));

            let pending = contact_for(&peer_key, now - 10, Some(&signer));
            book.insert(CheckedPeerContact::check(
                pending,
                &NoopValidatorClaimVerifier,
            ));

            // The peer signs a new timestamp with its network key but reuses the old validator
            // signature. Its outer signature is valid, while the current validator claim is not.
            let mut refreshed = first.inner.clone();
            refreshed.timestamp = now;
            let refreshed = refreshed.sign(&peer_key);
            assert!(refreshed.verify());
            assert_eq!(
                refreshed.check_validator_claim(&verifier),
                Some(ValidatorVerification::Invalid(
                    InvalidReason::InvalidSignature
                ))
            );

            let budget = ValidatorClaimBudget::new(usize::from(check_on_insert));
            book.insert(CheckedPeerContact::check_with_budget(
                refreshed, &verifier, &budget,
            ));
            assert!(!book.get(&peer_id).unwrap().is_validator_verified());
            assert!(!book.get(&peer_id).unwrap().is_gossipable());

            if !check_on_insert {
                assert_eq!(
                    book.get_validator_peer_ids(signer.validator_address()),
                    vec![peer_id]
                );
                assert_eq!(
                    book.verified_claim_timestamps[signer.validator_address()][&peer_id],
                    now - 20
                );

                let pending = book.unverified_validator_contacts();
                assert_eq!(pending.len(), 1);
                let info = &pending[0];
                book.apply_validator_verifications([(
                    *info.peer_id(),
                    info.contact().timestamp(),
                    info.signed().check_validator_claim(&verifier).unwrap(),
                )]);
            }

            assert!(book
                .get_validator_peer_ids(signer.validator_address())
                .is_empty());
            assert!(!book
                .verified_claim_timestamps
                .contains_key(signer.validator_address()));
            assert!(book.unverified_validator_contacts().is_empty());
        }
    }

    #[test]
    fn inconclusive_checks_keep_the_original_verified_binding() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let budget = ValidatorClaimBudget::new(1);
        let contact = contact_for(&peer_key, now - 10, Some(&signer));
        let peer_id = contact.peer_id();
        book.insert(CheckedPeerContact::check_with_budget(
            contact, &verifier, &budget,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );

        // A freshly-checked contact (budget available) that comes back `Unverifiable` — e.g. our
        // own staking-contract state is transiently incomplete — must keep the old binding.
        let refreshed = contact_for(&peer_key, now, Some(&signer));
        book.insert(CheckedPeerContact::check(
            refreshed.clone(),
            &NoopValidatorClaimVerifier,
        ));

        book.apply_validator_verifications([(
            peer_id,
            now,
            refreshed
                .check_validator_claim(&NoopValidatorClaimVerifier)
                .unwrap(),
        )]);

        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()][&peer_id],
            now - 10
        );
        assert!(!book.get(&peer_id).unwrap().is_validator_verified());
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
        assert_eq!(book.unverified_validator_contacts().len(), 1);
    }

    #[test]
    fn pending_refresh_and_inconclusive_checks_do_not_extend_verified_binding_expiry() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        // Insert an expired contact directly so expiry can be checked without sleeping.
        let expired = contact_for(
            &peer_key,
            now - PeerContactBook::MAX_PEER_AGE - 10,
            Some(&signer),
        );
        let peer_id = expired.peer_id();
        book.insert(CheckedPeerContact::check(expired, &verifier));

        let refreshed = contact_for(&peer_key, now, Some(&signer));
        book.insert(CheckedPeerContact::check(
            refreshed.clone(),
            &NoopValidatorClaimVerifier,
        ));
        book.apply_validator_verifications([(
            peer_id,
            now,
            refreshed
                .check_validator_claim(&NoopValidatorClaimVerifier)
                .unwrap(),
        )]);

        // Resolution must filter the expired binding even before housekeeping runs.
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()][&peer_id],
            now - PeerContactBook::MAX_PEER_AGE - 10
        );
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());

        book.house_keeping();

        // The fresh pending contact remains dialable, but cannot keep the binding alive.
        assert_eq!(book.get(&peer_id).unwrap().signed(), &refreshed);
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
        assert_eq!(book.unverified_validator_contacts().len(), 1);
        assert!(!book
            .verified_claim_timestamps
            .contains_key(signer.validator_address()));
    }

    #[test]
    fn a_conclusively_invalid_claim_is_not_re_offered_to_the_recheck_sweep() {
        let (_own_key, mut book) = empty_book();
        let (signer, _verifier) = validator(1);
        // A verifier that does not know the claimed validator, so the claim is checked and
        // rejected outright as `Invalid` (here `UnknownValidator`; a bad signature is equivalent,
        // both are `ClaimCheck::Invalid`).
        let (_other_signer, other_verifier) = validator(2);
        let peer_key = Keypair::generate_ed25519();

        let contact = contact_for(&peer_key, now_secs(), Some(&signer));
        let peer_id = contact.peer_id();
        book.insert(CheckedPeerContact::check(contact, &other_verifier));

        // The bogus claim is stored but neither trusted nor indexed...
        let info = book.get(&peer_id).expect("contact must be stored");
        assert!(!info.is_validator_verified());
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());

        // ...and, crucially, it is NOT re-offered to the periodic re-check sweep. A definitive
        // `Invalid` is conclusive for this exact contact, so re-checking it would only waste the
        // sweep's bounded per-tick blockchain-read budget on a known-bad claim — which is exactly
        // what a peer attaching a bogus claim to every contact would try to exploit.
        assert!(book.unverified_validator_contacts().is_empty());
    }

    #[test]
    fn a_node_wide_unverifiable_check_exhausts_the_budget() {
        for reason in [
            UnverifiableReason::NoVerifier,
            UnverifiableReason::LightClient,
            UnverifiableReason::StateIncomplete,
        ] {
            let (signer, _verifier) = validator(1);
            let verifier = CountingVerifier::new(ValidatorVerification::Unverifiable(reason));
            let budget = ValidatorClaimBudget::with_refresh_reserve(2, 2);

            let first = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
            let checked = CheckedPeerContact::check_with_budget(first, &verifier, &budget);
            assert_eq!(checked.claim, ClaimCheck::Pending);
            assert_eq!(verifier.calls(), 1);

            // Every other claim would come back the same way, so the rest of the budget,
            // including the refresh reserve, is gone until the next tick, and the next contact is
            // left pending without being checked.
            let second = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
            let checked = CheckedPeerContact::check_with_budget(second, &verifier, &budget);
            assert_eq!(checked.claim, ClaimCheck::Pending);
            assert_eq!(verifier.calls(), 1);
            assert_eq!(budget.remaining(), (0, 0));

            // The next tick refills it.
            budget.reset();
            assert_eq!(budget.remaining(), (2, 2));
        }
    }

    #[test]
    fn conclusive_checks_do_not_exhaust_the_budget() {
        for (outcome, expected) in [
            (ValidatorVerification::Verified, ClaimCheck::Verified),
            (
                ValidatorVerification::Invalid(InvalidReason::UnknownValidator),
                ClaimCheck::Invalid,
            ),
            (
                ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
                ClaimCheck::Invalid,
            ),
        ] {
            let (signer, _verifier) = validator(1);
            let verifier = CountingVerifier::new(outcome);
            let budget = ValidatorClaimBudget::new(2);

            for calls in 1..=2 {
                let contact = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
                let checked = CheckedPeerContact::check_with_budget(contact, &verifier, &budget);
                assert_eq!(checked.claim, expected);
                assert_eq!(verifier.calls(), calls);
            }
            assert!(!budget.try_consume());
        }
    }

    #[test]
    fn a_refresh_of_a_verified_binding_is_verified_from_the_reserve_despite_a_flooded_budget() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let first = contact_for(&peer_key, now - 10, Some(&signer));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check(first, &verifier));
        assert!(book.has_live_verified_binding(&peer_id, signer.validator_address()));
        let book = RwLock::new(book);

        // Unauthenticated peers have used up the general budget with bogus claims under fresh
        // peer keys. Only the refresh reserve is left.
        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 1);
        let (bogus_signer, _) = validator(2);
        let flood = contact_for(&Keypair::generate_ed25519(), now, Some(&bogus_signer));
        assert_eq!(
            CheckedPeerContact::check_new_with_budget(flood, &book, &verifier, &budget).claim,
            ClaimCheck::Pending
        );

        // The validator's refreshed contact is still verified, so it stays gossipable and its
        // binding moves on to the new timestamp.
        let refreshed = contact_for(&peer_key, now, Some(&signer));
        let checked =
            CheckedPeerContact::check_new_with_budget(refreshed.clone(), &book, &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Verified);
        let mut book = book.into_inner();
        book.insert(checked);

        let info = book.get(&peer_id).unwrap();
        assert_eq!(info.signed(), &refreshed);
        assert!(info.is_validator_verified());
        assert!(info.is_gossipable());
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()][&peer_id],
            now
        );
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert_eq!(budget.remaining(), (0, 0));
    }

    #[test]
    fn a_peer_without_a_verified_binding_cannot_spend_the_reserve() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let now = now_secs();

        // The address is bound, but to another peer.
        let bound = contact_for(&Keypair::generate_ed25519(), now - 10, Some(&signer));
        let bound_peer_id = bound.peer_id();
        book.insert(CheckedPeerContact::check(bound, &verifier));
        assert!(book.has_live_verified_binding(&bound_peer_id, signer.validator_address()));

        // A peer whose earlier contact claimed the address but was never conclusively checked.
        let pending_key = Keypair::generate_ed25519();
        let pending = contact_for(&pending_key, now - 10, Some(&signer));
        book.insert(CheckedPeerContact::pending(pending));
        let book = RwLock::new(book);

        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 3);
        for contact in [
            // A peer we have never heard of, claiming the address with a genuine claim that would
            // verify.
            contact_for(&Keypair::generate_ed25519(), now, Some(&signer)),
            contact_for(&pending_key, now, Some(&signer)),
        ] {
            assert!(!book
                .read()
                .has_live_verified_binding(&contact.peer_id(), signer.validator_address()));
            assert_eq!(
                CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget).claim,
                ClaimCheck::Pending
            );
        }
        assert_eq!(budget.remaining(), (0, 3));
    }

    #[test]
    fn a_refresh_claiming_another_address_does_not_use_the_reserve() {
        let (_own_key, mut book) = empty_book();
        let (signer_a, verifier_a) = validator(1);
        let (signer_b, verifier_b) = validator(2);
        let verifier = TestVerifier {
            keys: verifier_a.keys.into_iter().chain(verifier_b.keys).collect(),
        };
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let first = contact_for(&peer_key, now - 10, Some(&signer_a));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check(first, &verifier));
        let book = RwLock::new(book);

        // The binding is to the first address only. Claiming another one is a fresh claim, and
        // has to compete for the general budget like any other.
        let switched = contact_for(&peer_key, now, Some(&signer_b));
        assert!(!book
            .read()
            .has_live_verified_binding(&peer_id, signer_b.validator_address()));
        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 1);
        assert_eq!(
            CheckedPeerContact::check_new_with_budget(switched, &book, &verifier, &budget).claim,
            ClaimCheck::Pending
        );
        assert_eq!(budget.remaining(), (0, 1));
    }

    #[test]
    fn an_expired_binding_does_not_unlock_the_reserve() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        // `insert` does not check the age, so this creates a binding that has already expired,
        // as it would be until the next house-keeping removes it.
        let expired = contact_for(
            &peer_key,
            now - PeerContactBook::MAX_PEER_AGE - 10,
            Some(&signer),
        );
        let peer_id = expired.peer_id();
        book.insert(CheckedPeerContact::check(expired, &verifier));
        assert!(book.verified_claim_timestamps[signer.validator_address()].contains_key(&peer_id));
        assert!(!book.has_live_verified_binding(&peer_id, signer.validator_address()));
        let book = RwLock::new(book);

        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 1);
        let refreshed = contact_for(&peer_key, now, Some(&signer));
        assert_eq!(
            CheckedPeerContact::check_new_with_budget(refreshed, &book, &verifier, &budget).claim,
            ClaimCheck::Pending
        );
        assert_eq!(budget.remaining(), (0, 1));
    }

    #[test]
    fn a_refresh_relayed_several_times_takes_one_unit_of_the_reserve() {
        let (signer, verifier) = validator(1);
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let counting = CountingVerifier::new(ValidatorVerification::Verified);
        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 3);

        // Copies of the validator's genuine refreshed contact, relayed to us several times in one
        // batch or on several connections at once. All of them are checked before any is stored,
        // so every one of them is new to the book and refreshes a live binding.
        let refreshed = contact_for(&peer_key, now_secs(), Some(&signer));
        let claims: Vec<_> = (0..3)
            .map(|_| {
                CheckedPeerContact::check_new_with_budget(
                    refreshed.clone(),
                    &book,
                    &counting,
                    &budget,
                )
                .claim
            })
            .collect();

        // Only the first copy is charged to the reserve and checked. The others fall back to the
        // general budget, which is gone, so the reserve is left for other validators.
        assert_eq!(
            claims,
            vec![
                ClaimCheck::Verified,
                ClaimCheck::Pending,
                ClaimCheck::Pending
            ]
        );
        assert_eq!(counting.calls(), 1);
        assert_eq!(budget.remaining(), (0, 2));

        // A refresh of another validator still gets its unit.
        let (other_signer, other_verifier) = validator(2);
        let (other_key, other_book) = book_with_binding(&other_signer, &other_verifier);
        let other_refreshed = contact_for(&other_key, now_secs(), Some(&other_signer));
        assert_eq!(
            CheckedPeerContact::check_new_with_budget(
                other_refreshed,
                &other_book,
                &counting,
                &budget
            )
            .claim,
            ClaimCheck::Verified
        );
        assert_eq!(budget.remaining(), (0, 1));
    }

    #[test]
    fn a_verified_copy_settles_a_pending_copy_stored_before_it() {
        let (signer, verifier) = validator(1);
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 1);

        // Two copies of the same refresh are checked at once. Only the first one gets the
        // validator's unit of the reserve, and the other is left pending...
        let now = now_secs();
        let refreshed = contact_for(&peer_key, now, Some(&signer));
        let peer_id = refreshed.peer_id();
        let verified =
            CheckedPeerContact::check_new_with_budget(refreshed.clone(), &book, &verifier, &budget);
        let pending =
            CheckedPeerContact::check_new_with_budget(refreshed.clone(), &book, &verifier, &budget);
        assert_eq!(verified.claim, ClaimCheck::Verified);
        assert_eq!(pending.claim, ClaimCheck::Pending);

        // ...but stored first, which keeps the binding at its old timestamp.
        let mut book = book.into_inner();
        book.insert(pending);
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()][&peer_id],
            now - 10
        );

        // The verified copy then settles the claim, just like the re-check sweep would.
        book.insert(verified);
        let info = book.get(&peer_id).unwrap();
        assert_eq!(info.signed(), &refreshed);
        assert!(info.is_validator_verified());
        assert!(info.is_gossipable());
        assert!(book.unverified_validator_contacts().is_empty());
        assert_eq!(
            book.verified_claim_timestamps[signer.validator_address()][&peer_id],
            now
        );
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
    }

    #[test]
    fn a_rejected_copy_settles_a_pending_copy_stored_before_it() {
        let (signer, verifier) = validator(1);
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let mut book = book.into_inner();

        let refreshed = contact_for(&peer_key, now_secs(), Some(&signer));
        let peer_id = refreshed.peer_id();
        book.insert(CheckedPeerContact::pending(refreshed.clone()));
        assert!(book.has_live_verified_binding(&peer_id, signer.validator_address()));

        // A copy that was checked against a staking contract that no longer knows the validator.
        let (_, unaware_verifier) = validator(2);
        let rejected = CheckedPeerContact::check(refreshed, &unaware_verifier);
        assert_eq!(rejected.claim, ClaimCheck::Invalid);
        book.insert(rejected);

        // As with a rejection by the re-check sweep, the binding is gone and the claim is not
        // re-checked again.
        let info = book.get(&peer_id).unwrap();
        assert!(!info.is_validator_verified());
        assert!(!info.is_gossipable());
        assert!(!info.validator_claim_needs_recheck());
        assert!(!book.has_live_verified_binding(&peer_id, signer.validator_address()));
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
    }

    #[test]
    fn a_copy_only_settles_an_identical_contact_whose_claim_is_pending() {
        let (_own_key, mut book) = empty_book();
        let (signer_a, verifier_a) = validator(1);
        let (signer_b, verifier_b) = validator(2);
        let both = TestVerifier {
            keys: verifier_a
                .keys
                .clone()
                .into_iter()
                .chain(verifier_b.keys.clone())
                .collect(),
        };
        let now = now_secs();

        // A contact with the same peer and timestamp, but another claim, settles nothing.
        let peer_key = Keypair::generate_ed25519();
        let pending = contact_for(&peer_key, now, Some(&signer_a));
        let peer_id = pending.peer_id();
        book.insert(CheckedPeerContact::pending(pending.clone()));
        let other_claim = contact_for(&peer_key, now, Some(&signer_b));
        let other_claim = CheckedPeerContact::check(other_claim, &both);
        assert_eq!(other_claim.claim, ClaimCheck::Verified);
        book.insert(other_claim);
        let info = book.get(&peer_id).unwrap();
        assert_eq!(info.signed(), &pending);
        assert!(info.validator_claim_needs_recheck());
        assert!(!info.is_validator_verified());
        assert!(!book.has_live_verified_binding(&peer_id, signer_a.validator_address()));
        assert!(!book.has_live_verified_binding(&peer_id, signer_b.validator_address()));

        // A claim that was verified stays verified, whatever a copy checked later says...
        let verified_key = Keypair::generate_ed25519();
        let verified = contact_for(&verified_key, now, Some(&signer_a));
        let verified_peer_id = verified.peer_id();
        book.insert(CheckedPeerContact::check(verified.clone(), &verifier_a));
        book.insert(CheckedPeerContact::check(verified, &verifier_b));
        assert!(book.get(&verified_peer_id).unwrap().is_validator_verified());
        assert!(book.has_live_verified_binding(&verified_peer_id, signer_a.validator_address()));

        // ...and a claim that was rejected stays rejected.
        let rejected_key = Keypair::generate_ed25519();
        let rejected = contact_for(&rejected_key, now, Some(&signer_a));
        let rejected_peer_id = rejected.peer_id();
        book.insert(CheckedPeerContact::check(rejected.clone(), &verifier_b));
        book.insert(CheckedPeerContact::check(rejected, &verifier_a));
        assert!(!book.get(&rejected_peer_id).unwrap().is_validator_verified());
        assert!(!book.has_live_verified_binding(&rejected_peer_id, signer_a.validator_address()));
    }

    #[test]
    fn claims_are_checked_without_holding_the_book_lock() {
        let (signer, verifier) = validator(1);
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let checking = LockCheckingVerifier {
            book: &book,
            inner: &verifier,
        };
        let budget = ValidatorClaimBudget::with_refresh_reserve(1, 1);

        // A refresh of a live binding, charged to the reserve...
        let refreshed = contact_for(&peer_key, now_secs(), Some(&signer));
        assert_eq!(
            CheckedPeerContact::check_new_with_budget(refreshed, &book, &checking, &budget).claim,
            ClaimCheck::Verified
        );
        assert_eq!(budget.remaining(), (1, 0));

        // ...and a claim of a peer we have never heard of, charged to the general budget.
        let fresh = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
        assert_eq!(
            CheckedPeerContact::check_new_with_budget(fresh, &book, &checking, &budget).claim,
            ClaimCheck::Verified
        );
        assert_eq!(budget.remaining(), (0, 0));
    }

    #[test]
    fn a_node_wide_unverifiable_check_of_a_new_contact_exhausts_the_budget() {
        let (signer, verifier) = validator(1);
        let incomplete = CountingVerifier::new(ValidatorVerification::Unverifiable(
            UnverifiableReason::StateIncomplete,
        ));

        // A refresh of a live binding, charged to the reserve, and a claim of a peer we have never
        // heard of, charged to the general budget, each empty the whole budget.
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let refreshed = contact_for(&peer_key, now_secs(), Some(&signer));
        let fresh = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
        for (calls, contact) in [refreshed, fresh].into_iter().enumerate() {
            let budget = ValidatorClaimBudget::with_refresh_reserve(2, 2);
            assert_eq!(
                CheckedPeerContact::check_new_with_budget(contact, &book, &incomplete, &budget)
                    .claim,
                ClaimCheck::Pending
            );
            assert_eq!(incomplete.calls(), calls + 1);
            assert_eq!(budget.remaining(), (0, 0));

            // The next new contact is left pending without being checked.
            let next = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
            assert_eq!(
                CheckedPeerContact::check_new_with_budget(next, &book, &incomplete, &budget).claim,
                ClaimCheck::Pending
            );
            assert_eq!(incomplete.calls(), calls + 1);
        }
    }

    #[test]
    fn check_with_budget_never_spends_the_refresh_reserve() {
        let (signer, verifier) = validator(1);
        let counting = CountingVerifier::new(ValidatorVerification::Verified);
        let budget = ValidatorClaimBudget::with_refresh_reserve(0, 1);

        // Even a contact of a peer that holds a live binding is only charged to the general
        // budget here.
        let (peer_key, _book) = book_with_binding(&signer, &verifier);
        let refreshed = contact_for(&peer_key, now_secs(), Some(&signer));
        assert_eq!(
            CheckedPeerContact::check_with_budget(refreshed, &counting, &budget).claim,
            ClaimCheck::Pending
        );
        assert_eq!(counting.calls(), 0);
        assert_eq!(budget.remaining(), (0, 1));
    }
    /// A signing key other than the one the verifier of `validator(seed)` knows, e.g. the key the
    /// validator rotated to, or one an attacker made up.
    fn other_signing_key() -> KeyPair {
        let mut rng = test_rng(false);
        let _ = KeyPair::generate(&mut rng);
        KeyPair::generate(&mut rng)
    }

    #[test]
    fn a_validator_keeps_only_its_newest_bindings() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let now = now_secs();
        let extra = 2;

        let peers: Vec<(Keypair, u64)> = (0..PeerContactBook::MAX_BINDINGS_PER_VALIDATOR + extra)
            .map(|i| (Keypair::generate_ed25519(), now - 100 + i as u64))
            .collect();
        for (key, timestamp) in &peers {
            book.insert(CheckedPeerContact::check(
                contact_for(key, *timestamp, Some(&signer)),
                &verifier,
            ));
        }

        // Only the newest ones are reported...
        let expected: Vec<PeerId> = peers
            .iter()
            .rev()
            .take(PeerContactBook::MAX_BINDINGS_PER_VALIDATOR)
            .map(|(key, _)| key.public().to_peer_id())
            .collect();
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            expected
        );
        // ...and the contacts of the others are neither gossiped nor offered for a re-check.
        for (key, _) in peers.iter().take(extra) {
            let info = book.get(&key.public().to_peer_id()).unwrap();
            assert!(!info.is_validator_verified());
            assert!(!info.is_gossipable());
            assert!(!info.validator_claim_needs_recheck());
        }

        // A binding older than all of them does not make it either.
        let old_key = Keypair::generate_ed25519();
        book.insert(CheckedPeerContact::check(
            contact_for(&old_key, now - 1_000, Some(&signer)),
            &verifier,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            expected
        );
        assert!(!book
            .get(&old_key.public().to_peer_id())
            .unwrap()
            .is_gossipable());
    }

    #[test]
    fn a_validator_verifies_only_its_share_of_contacts_per_tick() {
        let (_own_key, book) = empty_book();
        let book = RwLock::new(book);
        let (signer, verifier) = validator(1);
        let budget =
            ValidatorClaimBudget::with_refresh_reserve(256, 256).with_per_validator_limit(2);
        let now = now_secs();

        // Bogus claims to the validator's address do not use up its share.
        let bogus =
            ValidatorClaimSigner::new(signer.validator_address().clone(), other_signing_key());
        for _ in 0..3 {
            let contact = contact_for(&Keypair::generate_ed25519(), now, Some(&bogus));
            let checked =
                CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget);
            assert_eq!(checked.claim, ClaimCheck::Invalid);
            book.write().insert(checked);
        }

        // Its genuine claims do, and once it is used up, further ones are left pending without
        // spending any budget.
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let contact = contact_for(&Keypair::generate_ed25519(), now, Some(&signer));
            let checked =
                CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget);
            outcomes.push(checked.claim);
            book.write().insert(checked);
        }
        assert_eq!(
            outcomes,
            vec![
                ClaimCheck::Verified,
                ClaimCheck::Verified,
                ClaimCheck::Pending,
                ClaimCheck::Pending
            ]
        );
        assert_eq!(budget.remaining(), (256 - 5, 256));

        // Another validator is not affected.
        let (other_signer, other_verifier) = validator(2);
        let contact = contact_for(&Keypair::generate_ed25519(), now, Some(&other_signer));
        let checked =
            CheckedPeerContact::check_new_with_budget(contact, &book, &other_verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Verified);
    }

    #[test]
    fn a_stale_verification_does_not_undo_a_later_rejection() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier_before_rotation) = validator(1);
        let address = signer.validator_address().clone();
        let verifier_after_rotation = TestVerifier {
            keys: HashMap::from([(address.clone(), other_signing_key().public)]),
        };
        let peer_key = Keypair::generate_ed25519();
        let contact = contact_for(&peer_key, now_secs() - 5, Some(&signer));
        let peer_id = contact.peer_id();
        let timestamp = contact.inner.timestamp();

        // The re-check sweep checks the pending claim against the state before the rotation...
        book.insert(CheckedPeerContact::pending(contact.clone()));
        let stale = book.unverified_validator_contacts()[0]
            .signed()
            .check_validator_claim(&verifier_before_rotation)
            .unwrap();
        assert_eq!(stale, ValidatorVerification::Verified);

        // ...while an identical copy is rejected against the state after it, and settles the claim.
        book.insert(CheckedPeerContact::check(contact, &verifier_after_rotation));
        book.apply_validator_verifications([(peer_id, timestamp, stale)]);

        assert!(book.get_validator_peer_ids(&address).is_empty());
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
    }

    #[test]
    fn a_rejection_ends_a_verified_binding() {
        let (signer, verifier) = validator(1);
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let mut book = book.into_inner();
        let peer_id = peer_key.public().to_peer_id();
        let timestamp = book.get(&peer_id).unwrap().contact().timestamp();

        book.apply_validator_verifications([(
            peer_id,
            timestamp,
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
        )]);

        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
        assert!(!book.get(&peer_id).unwrap().is_gossipable());
    }

    #[test]
    fn a_contact_the_filter_would_drop_is_not_checked() {
        let (_own_key, book) = empty_book();
        let book = RwLock::new(book);
        let (signer, _verifier) = validator(1);
        let counting = CountingVerifier::new(ValidatorVerification::Verified);
        let budget = ValidatorClaimBudget::new(10);
        let filter = InsertFilter {
            services: Services::all(),
            only_secure_ws_connections: false,
        };

        // Too old for `insert_filtered`...
        let too_old = contact_for(
            &Keypair::generate_ed25519(),
            now_secs() - PeerContactBook::MAX_PEER_AGE - 10,
            Some(&signer),
        );
        // ...or without a secure websocket address, where one is required.
        let insecure = contact_for(&Keypair::generate_ed25519(), now_secs(), Some(&signer));
        let secure_only = InsertFilter {
            only_secure_ws_connections: true,
            ..filter
        };

        for (contact, filter) in [(too_old, &filter), (insecure, &secure_only)] {
            let checked = CheckedPeerContact::check_new_filtered_with_budget(
                contact, &book, &counting, &budget, filter,
            );
            assert_eq!(checked.claim, ClaimCheck::Pending);
        }
        assert_eq!(counting.calls(), 0);
        assert_eq!(budget.remaining(), (10, 0));
    }

    #[test]
    fn only_the_newest_contact_of_each_peer_in_a_batch_is_kept() {
        let (signer, _verifier) = validator(1);
        let (a, b) = (Keypair::generate_ed25519(), Keypair::generate_ed25519());
        let now = now_secs();
        let a_old = contact_for(&a, now - 20, Some(&signer));
        let a_new = contact_for(&a, now - 10, Some(&signer));
        let a_future = contact_for(&a, now + 1_000, Some(&signer));
        let b_only = contact_for(&b, now - 30, None);

        let kept = newest_contact_per_peer(vec![
            a_old.clone(),
            b_only.clone(),
            a_new.clone(),
            a_new.clone(),
            a_future,
        ]);

        assert_eq!(kept, vec![b_only, a_new]);
    }

    #[test]
    fn a_malformed_claim_is_rejected_without_charging_the_budget() {
        let (_own_key, book) = empty_book();
        let book = RwLock::new(book);
        let (signer, _verifier) = validator(1);
        let counting = CountingVerifier::new(ValidatorVerification::Verified);
        // No budget left at all, and no share of verified claims either.
        let budget = ValidatorClaimBudget::new(0).with_per_validator_limit(0);

        // Both too short and too long to be an Ed25519 signature.
        for size in [10, Ed25519Signature::SIZE + 1] {
            let keypair = Keypair::generate_ed25519();
            let mut contact = PeerContact::new(
                ["/ip4/127.0.0.1/tcp/8443".parse().unwrap()],
                keypair.public(),
                Services::all(),
                now_secs(),
            )
            .unwrap();
            contact.set_validator_info(Some(ValidatorInfo::new(
                signer.validator_address().clone(),
                TaggedSignature::from_bytes(vec![0; size]),
            )));
            let contact = contact.sign(&keypair);

            // It is conclusively rejected all the same, without asking the verifier.
            let checked =
                CheckedPeerContact::check_new_with_budget(contact, &book, &counting, &budget);
            assert_eq!(
                checked.claim,
                ClaimCheck::Invalid,
                "signature of {size} bytes"
            );
        }
        assert_eq!(counting.calls(), 0);
    }

    #[test]
    fn a_peer_verifies_only_one_contact_per_tick() {
        let (_own_key, book) = empty_book();
        let book = RwLock::new(book);
        let (signer, verifier) = validator(1);
        let budget =
            ValidatorClaimBudget::with_refresh_reserve(256, 256).with_per_validator_limit(2);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        let mut outcomes = Vec::new();
        for age in [30, 20, 10] {
            let contact = contact_for(&peer_key, now - age, Some(&signer));
            let checked =
                CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget);
            outcomes.push(checked.claim);
            book.write().insert(checked);
        }

        // The refreshes wait for the next tick. Meanwhile the first contact keeps the binding.
        assert_eq!(
            outcomes,
            vec![
                ClaimCheck::Verified,
                ClaimCheck::Pending,
                ClaimCheck::Pending
            ]
        );
        assert_eq!(
            book.read()
                .get_validator_peer_ids(signer.validator_address()),
            vec![peer_key.public().to_peer_id()]
        );
    }

    // Anyone can relay a validator's genuine older contacts. Relaying those of one of its peers,
    // oldest first so that each is new to us, must not use up the validator's share and keep the
    // contact of its current peer from being verified.
    #[test]
    fn replayed_contacts_of_one_peer_do_not_use_up_the_validators_share() {
        let (_own_key, book) = empty_book();
        let book = RwLock::new(book);
        let (signer, verifier) = validator(1);
        // The discovery behaviour's share.
        let budget =
            ValidatorClaimBudget::with_refresh_reserve(256, 256).with_per_validator_limit(4);
        let old_key = Keypair::generate_ed25519();
        let current_key = Keypair::generate_ed25519();
        let now = now_secs();

        for age in [50, 40, 30, 20, 10] {
            let contact = contact_for(&old_key, now - age, Some(&signer));
            let checked =
                CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget);
            book.write().insert(checked);
        }

        let contact = contact_for(&current_key, now, Some(&signer));
        let checked = CheckedPeerContact::check_new_with_budget(contact, &book, &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Verified);
        book.write().insert(checked);
        assert_eq!(
            book.read()
                .get_validator_peer_ids(signer.validator_address())
                .first(),
            Some(&current_key.public().to_peer_id())
        );
    }

    #[test]
    fn a_plain_contact_with_a_claim_is_stored_pending() {
        let (signer, verifier) = validator(1);
        let (peer_key, book) = book_with_binding(&signer, &verifier);
        let mut book = book.into_inner();
        let peer_id = peer_key.public().to_peer_id();

        // A refresh inserted without checking its claim keeps the binding and waits for the sweep.
        book.insert(contact_for(&peer_key, now_secs(), Some(&signer)));

        assert!(book.get(&peer_id).unwrap().validator_claim_needs_recheck());
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
    }

    #[test]
    fn stale_verified_bindings_are_offered_for_a_recheck() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let now = now_secs();

        let stale_key = Keypair::generate_ed25519();
        book.insert(CheckedPeerContact::check(
            contact_for(
                &stale_key,
                now - PeerContactBook::STALE_BINDING_AGE - 10,
                Some(&signer),
            ),
            &verifier,
        ));
        let fresh_key = Keypair::generate_ed25519();
        book.insert(CheckedPeerContact::check(
            contact_for(&fresh_key, now - 10, Some(&signer)),
            &verifier,
        ));
        let unverified_key = Keypair::generate_ed25519();
        book.insert(CheckedPeerContact::pending(contact_for(
            &unverified_key,
            now - PeerContactBook::STALE_BINDING_AGE - 10,
            Some(&signer),
        )));

        let stale: Vec<PeerId> = book
            .stale_verified_validator_contacts()
            .iter()
            .map(|info| *info.peer_id())
            .collect();
        assert_eq!(stale, vec![stale_key.public().to_peer_id()]);
    }
    #[test]
    fn a_stale_binding_kept_under_a_pending_refresh_is_offered_for_a_recheck() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let now = now_secs();
        let peer_key = Keypair::generate_ed25519();
        let peer_id = peer_key.public().to_peer_id();

        book.insert(CheckedPeerContact::check(
            contact_for(
                &peer_key,
                now - PeerContactBook::STALE_BINDING_AGE - 10,
                Some(&signer),
            ),
            &verifier,
        ));
        // A refresh that could not be checked keeps the binding at its old timestamp.
        let refresh = contact_for(&peer_key, now - 10, Some(&signer));
        let refresh_timestamp = refresh.inner.timestamp();
        book.insert(CheckedPeerContact::pending(refresh));
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );

        // The binding went stale, so the pending refresh is offered for a re-check...
        let stale: Vec<PeerId> = book
            .stale_verified_validator_contacts()
            .iter()
            .map(|info| *info.peer_id())
            .collect();
        assert_eq!(stale, vec![peer_id]);

        // ...and if it is rejected, e.g. because it is signed with a key rotated away, the
        // binding ends.
        book.apply_validator_verifications([(
            peer_id,
            refresh_timestamp,
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
        )]);
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
    }

    #[test]
    fn a_stale_binding_is_offered_for_a_recheck_once_in_a_while() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        book.insert(CheckedPeerContact::check(
            contact_for(
                &peer_key,
                now_secs() - PeerContactBook::STALE_BINDING_AGE - 10,
                Some(&signer),
            ),
            &verifier,
        ));
        let stale = book.stale_verified_validator_contacts();
        assert_eq!(stale.len(), 1);

        // Once picked, it is left alone until the stale age passed again.
        let unix_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        stale[0].mark_binding_rechecked(unix_time);
        assert!(book.stale_verified_validator_contacts().is_empty());

        stale[0].mark_binding_rechecked(
            unix_time - Duration::from_secs(PeerContactBook::STALE_BINDING_AGE + 1),
        );
        assert_eq!(book.stale_verified_validator_contacts().len(), 1);
    }
}
