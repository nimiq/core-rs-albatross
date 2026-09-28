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
    InvalidReason, SignedValidatorClaim, ValidatorClaimVerifier, ValidatorVerification,
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

    /// Whether this contact carries a validator claim whose signature is malformed, i.e. does not
    /// even have the size of an Ed25519 signature. Such a claim can never be honest, so it is
    /// rejected without spending any budget on it, and a relayed contact carrying one is not
    /// stored at all (see [`PeerContactBook::insert_filtered`]).
    pub fn has_malformed_validator_claim(&self) -> bool {
        self.validator_info
            .as_ref()
            .is_some_and(|info| !info.has_well_formed_signature())
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
}

/// The verification status of a [`SignedPeerContact`]'s validator claim.
///
/// Contacts received from peers get it from [`CheckedPeerContact::check`], which checks the
/// claim, or from [`CheckedPeerContact::pending`], which leaves it [`Pending`](Self::Pending)
/// unchecked. The pending claim of a stored contact is settled later by the periodic re-check
/// sweep (see [`PeerContactBook::unverified_validator_contacts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimCheck {
    /// This contact carries no validator claim.
    None,
    /// The claim was cryptographically checked against this exact contact and holds.
    Verified,
    /// The claim was cryptographically checked against this exact contact and does not hold.
    Invalid,
    /// This exact contact's claim was not conclusively checked: it was not checked at all, or
    /// the check came back [`ValidatorVerification::Unverifiable`]. The contact stays unverified
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
    /// Checks the contact's validator claim, if it has one.
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

/// The filter that [`PeerContactBook::insert_filtered`] applies to contacts relayed to us.
///
/// Besides the fields below, a contact that carries a validator claim whose signature is
/// malformed is always filtered out; see [`PeerContactBook::insert_filtered`].
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
    /// Whether this contact still needs a conclusive check: verification was skipped for budget,
    /// returned [`ValidatorVerification::Unverifiable`], or a conclusive check of this contact
    /// disagreed with the outcome it was settled with (see
    /// [`PeerContactInfo::reopen_validator_claim`]). Otherwise, both verified and invalid claims
    /// are excluded from the periodic re-check sweep.
    validator_verification_pending: bool,
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
    /// dialable; only its rejected claim is left alone until the peer advertises a newer contact,
    /// which is checked anew on arrival.
    ///
    /// The one exception is a claim whose two conclusive checks disagreed, e.g. because a block
    /// that registered the validator or rotated its signing key landed between them, or because
    /// its binding was evicted in between: it is re-opened for one more check (see
    /// [`Self::reopen_validator_claim`]). That takes a check that verified, so it cannot be
    /// brought about without the validator's signing key.
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

    /// Re-opens the claim of the current contact for the re-check sweep, after a conclusive check
    /// of it disagreed with the outcome it was settled with: the contact is no longer considered
    /// verified, and its claim is pending again. See `PeerContactBook::settle_or_reopen_claim`.
    pub(crate) fn reopen_validator_claim(&self) {
        let mut meta = self.meta.write();
        meta.validator_verified = false;
        meta.validator_verification_pending = true;
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

    /// Whether [`Self::insert_filtered`] keeps `peer_contact` under `filter`: it provides the
    /// services we need (or both we and it are validators), advertises a secure websocket address
    /// if we require one, is neither from the future nor older than [`Self::MAX_PEER_AGE`], and
    /// does not carry a validator claim whose signature is malformed.
    fn passes_filter(&self, peer_contact: &PeerContact, filter: &InsertFilter) -> bool {
        // A claim whose signature does not even have the size of one can never be honest: every
        // validator signs its claims with an Ed25519 key. Such a contact is not worth storing, let
        // alone the bytes of that signature, which are bounded by the message size only. The
        // contact is dropped silently, like any other contact that does not pass this filter,
        // rather than failing its signature check, since nodes that do not know about validator
        // claims relay such contacts like any other, and must not be disconnected for it.
        if peer_contact.has_malformed_validator_claim() {
            return false;
        }

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
    fn store(&mut self, checked: CheckedPeerContact) {
        let peer_id = checked.contact.inner.peer_id();
        let existing = self.peer_contacts.get(&peer_id).cloned();

        if let Some(existing) = &existing {
            // Only update the contact if the timestamp is greater than the entry we have
            if existing.contact().timestamp >= checked.contact.inner.timestamp {
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

    /// Applies the conclusive outcome `verified` of a check of the current contact `info` to its
    /// claim.
    ///
    /// While the claim is pending, the outcome settles it: a verification establishes or renews
    /// the peer's binding at the contact's timestamp, and a rejection removes the binding. Once
    /// the claim is settled, an outcome that agrees with it changes nothing.
    ///
    /// An outcome that disagrees with it re-opens the claim for the re-check sweep (see
    /// [`PeerContactInfo::reopen_validator_claim`]) and removes the binding in the meantime. The
    /// two checks were computed outside the lock, in either order, and read blockchain state at
    /// different times, e.g. before and after a block that registered the validator or rotated
    /// its signing key landed. This cannot tell which of them read the newer state, so the
    /// binding is withdrawn, which is the safe side, and the claim is checked once more by the
    /// next sweep, which reads the state current by then. That check settles it, since a pending
    /// claim never disagrees with anything. Only a check that verified can bring this about, so
    /// it takes the validator's signing key, and a claim that verified once is re-opened at most
    /// once per conclusive check of it, each of which is bounded like any other check: by the
    /// claim budget on arrival, and by the sweep's cap and per-validator share on a re-check.
    ///
    /// A contact whose binding was evicted, because its validator has newer ones (see
    /// [`Self::index_add`]), is settled and unverified as well, so a verification of it that was
    /// computed before the eviction re-opens it just the same. The re-check then binds it again
    /// if there is room by then, or evicts it again, which settles it.
    fn settle_or_reopen_claim(&mut self, info: &PeerContactInfo, verified: bool) {
        // A contact without a claim has nothing to settle. The sweep never checks one, but this
        // guards the public `apply_validator_verifications` all the same.
        if info.validator_address().is_none() {
            return;
        }
        let peer_id = info.peer_id;
        let validator_address = info.validator_address().cloned();
        if info.validator_claim_needs_recheck() {
            info.set_validator_verified(verified);
            if verified {
                debug!(%peer_id, ?validator_address, "Verified validator claim of peer contact");
                self.index_add(info);
            } else {
                let removed_binding = self.index_remove(info);
                debug!(
                    %peer_id,
                    ?validator_address,
                    removed_binding,
                    "Rejected validator claim of peer contact",
                );
            }
        } else if info.is_validator_verified() == verified {
            trace!(%peer_id, "Ignoring a check of a validator claim that was settled the same way since");
        } else {
            info.reopen_validator_claim();
            let removed_binding = self.index_remove(info);
            debug!(
                %peer_id,
                ?validator_address,
                verified,
                removed_binding,
                "A check of a validator claim disagreed with the one that settled it, checking it once more",
            );
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
    ///
    /// A contact carrying a validator claim whose signature is malformed is ignored as well (see
    /// `passes_filter`). This is for contacts relayed to us by other peers, which may
    /// hand us any number of them. The contact a peer presents for itself in its handshake goes
    /// through [`Self::insert`] and is stored regardless of its claim, so that the peer can be
    /// dialed again; there is one such contact per connection.
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

    /// Snapshot of the contacts whose validator claim still needs a fresh cryptographic check —
    /// it has not been conclusively checked yet (skipped for budget or came back
    /// [`ValidatorVerification::Unverifiable`]). Claims that were conclusively checked —
    /// `Verified` or `Invalid` — are not included, so a peer cannot keep the sweep busy by
    /// attaching claims that are rejected outright.
    ///
    /// The result is ordered by peer ID rather than left in arbitrary hash-map order, so that a
    /// caller re-checking only a bounded number of claims each tick (see
    /// [`super::behaviour::Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK`]) can find the contact it
    /// stopped at in the next snapshot and continue from there, and still reach every contact,
    /// instead of always favoring whichever contacts happen to land first in `HashMap` iteration
    /// order. The caller relies on this order to find contacts by peer ID.
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

    /// Applies validator claim checks computed outside the lock.
    ///
    /// Each result carries the contact timestamp it was computed from; results for a contact
    /// that has since been replaced are discarded. A conclusive outcome settles the contact's
    /// claim if it is still pending: a verification establishes or refreshes the peer's verified
    /// binding at the checked contact's timestamp, and a rejection removes the binding. A
    /// rejection also removes the binding of a contact that was verified before, so that a claim
    /// that no longer holds, e.g. after the validator rotated its signing key, does not outlive
    /// the check that found out (see [`Self::stale_verified_validator_contacts`]); the claim is
    /// then checked once more. Likewise if the claim was settled the other way in the meantime,
    /// e.g. by an identical copy checked on arrival (see [`Self::store`]): the two checks may have
    /// read blockchain state in either order, so the binding is removed and the claim is
    /// re-opened for the next sweep (see `settle_or_reopen_claim`). An inconclusive result leaves
    /// the binding's original expiry intact.
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
            let info = Arc::clone(info);

            match verification {
                ValidatorVerification::Verified => self.settle_or_reopen_claim(&info, true),
                ValidatorVerification::Invalid(reason) => {
                    trace!(%peer_id, ?reason, "Validator claim of peer contact failed a re-check");
                    self.settle_or_reopen_claim(&info, false);
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
    /// for the next house-keeping tick. Returns whether the re-signed contact is no newer than the
    /// one it replaced, which is the case if our clock still reads the second that one was signed
    /// in (contact timestamps are in seconds), or an earlier one because it stepped back since.
    /// Peers that already stored the replaced contact discard the re-signed one, since they only
    /// store a strictly newer contact, so the caller should re-sign our contact once more with
    /// [`Self::update_own_contact`] once our clock reads a later second than the replaced
    /// contact's timestamp. Giving the re-signed contact a later timestamp instead is not an
    /// option: peers reject contacts from the future.
    pub fn set_validator_claim_signer(
        &mut self,
        signer: Option<ValidatorClaimSigner>,
        keypair: &Keypair,
    ) -> bool {
        let replaced_timestamp = self.own_peer_contact.contact().timestamp();
        self.validator_claim_signer = signer;
        self.update_own_contact(keypair);
        self.own_peer_contact.contact().timestamp() <= replaced_timestamp
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

            // A newer pending contact must not keep an older verified binding alive.
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
    use nimiq_keys::{Ed25519PublicKey, SecureGenerate};
    use nimiq_network_interface::validator_record::ValidatorRecord;
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;

    use super::*;
    use crate::discovery::validator_verifier::{InvalidReason, NoopValidatorClaimVerifier};

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
    fn a_new_signer_reports_whether_our_resigned_contact_is_newer() {
        let (signer, _verifier) = validator(1);
        let book_signed_at = |timestamp| {
            let own_key = Keypair::generate_ed25519();
            let book =
                PeerContactBook::new(contact_for(&own_key, timestamp, None), false, true, true);
            (own_key, book)
        };

        // Signed in an earlier second, the re-signed contact is newer and replaces the old one at
        // peers that stored it.
        let (own_key, mut book) = book_signed_at(now_secs() - 10);
        assert!(!book.set_validator_claim_signer(Some(signer.clone()), &own_key));

        // Signed within the same second, it is not. Try again if the second ends in between.
        for attempt in 0.. {
            let timestamp = now_secs();
            let (own_key, mut book) = book_signed_at(timestamp);
            let resign_again = book.set_validator_claim_signer(Some(signer.clone()), &own_key);
            if book.get_own_contact().contact().timestamp() == timestamp {
                assert!(resign_again);
                break;
            }
            assert!(attempt < 10, "never re-signed within the same second");
        }

        // Nor is it if our clock stepped back. The signer is installed all the same.
        let (own_key, mut book) = book_signed_at(now_secs() + 60);
        assert!(book.set_validator_claim_signer(Some(signer.clone()), &own_key));
        assert_eq!(
            book.get_own_contact().contact().validator_address(),
            Some(signer.validator_address())
        );
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
}
