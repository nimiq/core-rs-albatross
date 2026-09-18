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
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::{
    network::Network as NetworkInterface,
    peer_info::{PeerInfo, Services},
    validator_record::{ValidatorRecord, ValidatorRecordSigner},
};
use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSignable, TaggedSignature, TaggedSigned};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::validator_verifier::{
    SignedValidatorRecord, ValidatorClaimBudget, ValidatorRecordVerifier, ValidatorVerification,
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
/// such that a [ValidatorRecord] can be constructed. Importantly this also includes the signature.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatorInfo {
    /// The address of the validator. This is the unique identifier for a validator.
    validator_address: Address,

    /// The signature for the [ValidatorRecord].
    /// It does _not_ verify for this structure, but only once the [nimiq_utils::tagged_signing::TaggedSigned] is reconstructed
    /// with the given information of this struct and the corresponding [PeerContact].
    signature: TaggedSignature<ValidatorRecord<<Network as NetworkInterface>::PeerId>, KeyPair>,
}

impl ValidatorInfo {
    pub fn new(
        validator_address: Address,
        signature: TaggedSignature<ValidatorRecord<<Network as NetworkInterface>::PeerId>, KeyPair>,
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

    /// The signature over the corresponding [`ValidatorRecord`].
    pub fn signature(
        &self,
    ) -> &TaggedSignature<ValidatorRecord<<Network as NetworkInterface>::PeerId>, KeyPair> {
        &self.signature
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
        self.validator_info.as_ref().map(ValidatorInfo::validator_address)
    }

    /// Attaches (or removes) the validator claim of this contact.
    ///
    /// This invalidates any existing signature over the contact, so it must be called before
    /// signing.
    pub fn set_validator_info(&mut self, validator_info: Option<ValidatorInfo>) {
        self.validator_info = validator_info;
    }

    /// Reconstructs the signed [`ValidatorRecord`] this contact claims, if any.
    ///
    /// The record binds the contact's peer ID and timestamp to the validator address, so it is
    /// only meaningful together with the contact it was taken from. Note that the timestamp is in
    /// seconds here, whereas DHT records use milliseconds.
    pub fn signed_validator_record(&self) -> Option<SignedValidatorRecord> {
        let validator_info = self.validator_info.as_ref()?;
        let record = ValidatorRecord::new(
            self.peer_id(),
            validator_info.validator_address.clone(),
            self.timestamp,
        );
        Some(TaggedSigned::new(record, validator_info.signature.clone()))
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
    pub fn check_validator_claim(
        &self,
        verifier: &dyn ValidatorRecordVerifier,
    ) -> Option<ValidatorVerification> {
        let signed_record = self.inner.signed_validator_record()?;
        Some(verifier.verify_validator_record(&signed_record))
    }
}

/// The verification status of a [`SignedPeerContact`]'s validator claim, as determined by
/// [`CheckedPeerContact::check`] or [`CheckedPeerContact::check_with_budget`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimCheck {
    /// This contact carries no validator claim.
    None,
    /// The claim was cryptographically checked against this exact contact and holds.
    Verified,
    /// The claim was cryptographically checked against this exact contact and does not hold.
    Invalid,
    /// This exact contact's claim was not conclusively checked: budget was unavailable, or the
    /// check came back [`ValidatorVerification::Unverifiable`]. Whatever we currently believe
    /// about this peer's validator status must still be offered to the periodic re-check sweep
    /// rather than trusted indefinitely on this basis alone — see [`PeerContactBook::store`].
    Pending,
}

/// A [`SignedPeerContact`] together with the outcome of checking its validator claim.
///
/// The contact book indexes a contact under a validator address only if it arrives verified, and
/// the only way to produce that is [`CheckedPeerContact::check`]. Converting a plain
/// [`SignedPeerContact`] yields no claim, which is safe by default.
#[derive(Clone, Debug)]
pub struct CheckedPeerContact {
    contact: SignedPeerContact,
    claim: ClaimCheck,
}

impl CheckedPeerContact {
    /// Checks the contact's validator claim, if it has one.
    pub fn check(contact: SignedPeerContact, verifier: &dyn ValidatorRecordVerifier) -> Self {
        let claim = match contact.check_validator_claim(verifier) {
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

        Self { contact, claim }
    }

    /// The underlying signed contact.
    pub fn signed(&self) -> &SignedPeerContact {
        &self.contact
    }

    /// Checks the contact's validator claim if it has one and `budget` allows it; otherwise
    /// leaves it pending rather than actually checked.
    ///
    /// Verifying a claim reads blockchain state, so on hot, attacker-reachable paths (the
    /// discovery handshake and periodic peer-address updates) the number of checks performed
    /// must be bounded regardless of how many connections present a claim. A contact left
    /// pending here is picked up later by [`PeerContactBook::unverified_validator_contacts`].
    ///
    /// Pending (as opposed to a completed check) matters at store time: [`PeerContactBook`] must
    /// not let a check that never ran, or came back inconclusive, permanently downgrade *or*
    /// permanently upgrade what we believe about a validator's binding — see
    /// [`PeerContactBook::store`].
    pub fn check_with_budget(
        contact: SignedPeerContact,
        verifier: &dyn ValidatorRecordVerifier,
        budget: &ValidatorClaimBudget,
    ) -> Self {
        if contact.inner.validator_info().is_none() {
            return Self::from(contact);
        }
        if !budget.try_consume() {
            return Self {
                contact,
                claim: ClaimCheck::Pending,
            };
        }
        Self::check(contact, verifier)
    }
}

impl From<SignedPeerContact> for CheckedPeerContact {
    /// Treats the contact as carrying no validator claim.
    fn from(contact: SignedPeerContact) -> Self {
        Self {
            contact,
            claim: ClaimCheck::None,
        }
    }
}

/// Meta information attached to peer contact info objects. This is meant to be mutable and change over time.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PeerContactMeta {
    outer_protocol_address: Option<Multiaddr>,
    score: f64,
    /// Whether this contact's validator claim (if any) currently counts as trusted, for
    /// indexing/gossip purposes.
    validator_verified: bool,
    /// Whether `validator_verified` reflects an actual cryptographic check of *this* contact,
    /// rather than a belief carried forward from an earlier one (budget exhaustion, or an
    /// inconclusive check). A pending claim keeps being offered to the periodic re-check sweep
    /// even while `validator_verified` is `true`, so trust extended this way is never sustained
    /// indefinitely without ever being confirmed. See [`PeerContactBook::store`].
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
    /// Constructs a contact info with an explicit validator-claim belief.
    ///
    /// `validator_verified` is what we currently believe about the claim (if any); `pending`
    /// marks that belief as not (yet) confirmed by an actual check of *this* contact, so that
    /// [`PeerContactBook::unverified_validator_contacts`] keeps offering it for re-checking even
    /// while it is provisionally trusted. See [`PeerContactBook::store`].
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

    /// Whether this contact's validator claim currently counts as checked and holding.
    ///
    /// This can reflect a belief carried forward without an actual check of the current
    /// contact; use [`PeerContactInfo::validator_claim_needs_recheck`] to tell.
    pub fn is_validator_verified(&self) -> bool {
        self.meta.read().validator_verified
    }

    /// Whether this contact's validator claim should still be offered to the periodic re-check
    /// sweep ([`PeerContactBook::unverified_validator_contacts`]): either it isn't currently
    /// believed verified at all, or that belief was only carried forward from an earlier check
    /// rather than confirmed for this exact contact.
    pub(crate) fn validator_claim_needs_recheck(&self) -> bool {
        let meta = self.meta.read();
        !meta.validator_verified || meta.validator_verification_pending
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
    /// Peer IDs of contacts carrying a *verified* validator claim, indexed by validator address.
    ///
    /// This is kept in sync with `peer_contacts` and never contains our own peer ID.
    validator_peer_ids: HashMap<Address, HashSet<PeerId>>,
    /// Signs the validator record attached to our own contact, if we run a registered validator.
    validator_record_signer: Option<ValidatorRecordSigner>,
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
            validator_peer_ids: HashMap::new(),
            validator_record_signer: None,
            only_secure_addresses,
            allow_loopback_addresses,
            memory_transport,
        }
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
    /// This is the single place where `peer_contacts` is mutated on insert, so that
    /// `validator_peer_ids` stays a pure function of what is actually stored.
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

        // What we now believe about this contact's validator claim, and whether that belief is
        // backed by an actual check of *this* contact.
        //
        // A completed check (`Verified`/`Invalid`) always wins outright. A `Pending` outcome —
        // budget exhaustion, or an inconclusive result such as `Unverifiable` — must never be
        // treated as either a positive or a negative check of the *new* claim: silently
        // promoting unverified content would let a peer_id that was verified once keep changing
        // addresses/timestamp forever under the old verdict without ever being re-checked (e.g.
        // after the validator's signing key has since rotated away), and silently demoting would
        // knock a still-good validator out of `get_validator_peer_ids` merely because our own
        // state was transiently incomplete. So a carried-forward belief is always marked
        // `pending`, keeping it in `unverified_validator_contacts` until a real check confirms or
        // rejects it.
        let (validator_verified, pending) = match checked.claim {
            ClaimCheck::Verified => (true, false),
            ClaimCheck::Invalid | ClaimCheck::None => (false, false),
            ClaimCheck::Pending => {
                let carried = existing.as_ref().is_some_and(|prev| {
                    prev.is_validator_verified()
                        && prev.validator_address() == checked.contact.inner.validator_address()
                });
                (carried, true)
            }
        };

        let info = Arc::new(PeerContactInfo::new(
            checked.contact,
            validator_verified,
            pending,
        ));

        self.peer_contacts.insert(peer_id, Arc::clone(&info));

        // Drop the mapping the replaced contact established before adding the new one, so that a
        // peer that changed (or dropped) its validator address does not stay under the old one.
        if let Some(replaced) = existing {
            self.index_remove(&replaced);
        }
        self.index_add(&info);
    }

    /// Indexes a contact under its validator address, if it carries a verified claim.
    fn index_add(&mut self, info: &PeerContactInfo) {
        if !info.is_validator_verified() {
            return;
        }
        let Some(validator_address) = info.validator_address() else {
            return;
        };
        self.validator_peer_ids
            .entry(validator_address.clone())
            .or_default()
            .insert(info.peer_id);
    }

    /// Removes the mapping a contact established, if any.
    fn index_remove(&mut self, info: &PeerContactInfo) {
        let Some(validator_address) = info.validator_address() else {
            return;
        };
        if let Entry::Occupied(mut entry) = self.validator_peer_ids.entry(validator_address.clone())
        {
            entry.get_mut().remove(&info.peer_id);
            if entry.get().is_empty() {
                entry.remove();
            }
        }
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

        let peer_contact = &contact.signed().inner;

        // A peer is interesting to us in two cases:
        // - We are configured as a validator, and the peer is also a validator, then that peer is
        //   interesting regardless of the services that are provided by that peer.
        // - The services provided by the peer are a superset of the requested services.
        let we_are_validator = self
            .own_peer_contact
            .services()
            .contains(Services::VALIDATOR);
        let keep_validator = we_are_validator && peer_contact.services.contains(Services::VALIDATOR);
        if !keep_validator && !peer_contact.services.contains(services_filter) {
            return;
        }

        // Check that the peer provides secure ws addresses if required.
        if only_secure_ws_connections {
            let has_secure_ws_connections = peer_contact
                .addresses
                .iter()
                .any(utils::is_address_ws_secure);
            if !has_secure_ws_connections {
                return;
            }
        }

        let current_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Reject contacts with timestamps in the future
        if peer_contact.timestamp > current_ts {
            return;
        }

        // Rejects contacts that are older than the allowed age
        if contact_exceeds_age(
            peer_contact.timestamp,
            Duration::from_secs(PeerContactBook::MAX_PEER_AGE),
            Duration::from_secs(current_ts),
        ) {
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

    /// The peer IDs known to belong to `validator_address`, newest contact first.
    ///
    /// Only contacts whose validator claim this node verified are returned, and the stored contact
    /// is re-checked so that a stale index entry can never leak a peer that no longer claims this
    /// address.
    pub fn get_validator_peer_ids(&self, validator_address: &Address) -> Vec<PeerId> {
        let Some(peer_ids) = self.validator_peer_ids.get(validator_address) else {
            return Vec::new();
        };

        let mut contacts: Vec<&Arc<PeerContactInfo>> = peer_ids
            .iter()
            .filter_map(|peer_id| self.peer_contacts.get(peer_id))
            .filter(|info| {
                info.is_validator_verified() && info.validator_address() == Some(validator_address)
            })
            .collect();

        // Prefer the most recent claim: a validator that moved to another node should win over the
        // contact of the node it left behind.
        contacts.sort_unstable_by(|a, b| {
            b.contact()
                .timestamp
                .cmp(&a.contact().timestamp)
                .then_with(|| a.peer_id.cmp(&b.peer_id))
        });

        contacts.into_iter().map(|info| info.peer_id).collect()
    }

    /// Snapshot of the contacts whose validator claim still needs a fresh cryptographic check —
    /// either it has never been checked, or the current belief (verified or not) was only
    /// carried forward from an earlier one rather than confirmed for the contact now on file
    /// (see [`PeerContactInfo::validator_claim_needs_recheck`]).
    ///
    /// Verification needs the staking contract, so it must happen outside the contact book lock.
    /// Take this snapshot, check the claims, then feed the results back through
    /// [`PeerContactBook::apply_validator_verifications`].
    pub fn unverified_validator_contacts(&self) -> Vec<Arc<PeerContactInfo>> {
        self.peer_contacts
            .values()
            .filter(|info| {
                info.validator_address().is_some() && info.validator_claim_needs_recheck()
            })
            .cloned()
            .collect()
    }

    /// Applies validator claim checks computed outside the lock.
    ///
    /// Each result carries the contact timestamp it was computed from; results for a contact
    /// that has since been replaced are discarded. Unlike the check performed when a contact
    /// first arrives, a result here can *demote* an already-`verified` contact: entries reach
    /// this sweep specifically because their current status (if any) was never actually
    /// confirmed for the contact now on file, so a definitive `Invalid` here must override a
    /// merely carried-forward belief — this is what closes off indefinitely trusting a stale
    /// claim (e.g. from a peer identity that kept it alive across budget-exhausted refreshes
    /// after the validator's signing key has since rotated away).
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
                ValidatorVerification::Verified => {
                    debug!(%peer_id, validator_address = ?info.validator_address(), "Verified validator claim of peer contact");
                    info.set_validator_verified(true);
                    let info = Arc::clone(info);
                    self.index_add(&info);
                }
                ValidatorVerification::Invalid(reason) => {
                    let was_verified = info.is_validator_verified();
                    if was_verified {
                        debug!(%peer_id, ?reason, "Revoking a carried-forward validator claim that failed a definitive re-check");
                    }
                    info.set_validator_verified(false);
                    if was_verified {
                        let info = Arc::clone(info);
                        self.index_remove(&info);
                    }
                }
                // Inconclusive: leave whatever we currently believe untouched. It stays pending
                // (or unverified) and will be re-offered to the next sweep.
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
    pub fn set_validator_record_signer(
        &mut self,
        signer: Option<ValidatorRecordSigner>,
        keypair: &Keypair,
    ) {
        self.validator_record_signer = signer;
        self.update_own_contact(keypair);
    }

    /// Signs `contact` as our own contact, attaching a fresh validator claim if we have a signer.
    ///
    /// The claim covers the contact's timestamp, so it has to be produced here, every time the
    /// contact is (re-)signed, rather than being handed to us pre-computed.
    fn sign_own_contact(&mut self, mut contact: PeerContact, keypair: &Keypair) {
        let validator_info = self.validator_record_signer.as_ref().map(|signer| {
            let signed_record = signer.sign(contact.peer_id(), contact.timestamp());
            ValidatorInfo::new(
                signer.validator_address().clone(),
                signed_record.signature,
            )
        });
        contact.set_validator_info(validator_info);

        self.own_peer_contact = PeerContactInfo::from(contact.sign(keypair));
    }

    /// Gets our own contact information
    pub fn get_own_contact(&self) -> &PeerContactInfo {
        &self.own_peer_contact
    }

    /// Removes peer contacts that have already exceeded the maximum age as
    /// defined in `MAX_PEER_AGE`.
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
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;

    use super::*;
    use crate::discovery::validator_verifier::{InvalidReason, NoopValidatorRecordVerifier};

    /// A verifier that knows a fixed set of validator signing keys.
    struct TestVerifier {
        keys: HashMap<Address, Ed25519PublicKey>,
    }

    impl ValidatorRecordVerifier for TestVerifier {
        fn verify_validator_record(
            &self,
            signed_record: &SignedValidatorRecord,
        ) -> ValidatorVerification {
            let Some(public_key) = self.keys.get(&signed_record.record.validator_address) else {
                return ValidatorVerification::Invalid(InvalidReason::UnknownValidator);
            };
            if signed_record.verify(public_key) {
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
    fn validator(seed: u8) -> (ValidatorRecordSigner, TestVerifier) {
        let key_pair = KeyPair::generate(&mut test_rng(false));
        let mut address_bytes = [0u8; 20];
        address_bytes[0] = seed;
        let address = Address::from(address_bytes);

        let mut keys = HashMap::new();
        keys.insert(address.clone(), key_pair.public);

        (
            ValidatorRecordSigner::new(address, key_pair),
            TestVerifier { keys },
        )
    }

    fn contact_for(
        keypair: &Keypair,
        timestamp: u64,
        signer: Option<&ValidatorRecordSigner>,
    ) -> SignedPeerContact {
        let mut contact = PeerContact::new(
            ["/ip4/127.0.0.1/tcp/8443".parse().unwrap()],
            keypair.public(),
            Services::all(),
            timestamp,
        )
        .unwrap();

        if let Some(signer) = signer {
            let signed_record = signer.sign(contact.peer_id(), timestamp);
            contact.set_validator_info(Some(ValidatorInfo::new(
                signer.validator_address().clone(),
                signed_record.signature,
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
            &NoopValidatorRecordVerifier,
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
        book.insert(CheckedPeerContact::check(first, &verifier));

        // A newer contact from the same peer without any claim.
        let second = contact_for(&peer_key, now, None);
        book.insert(CheckedPeerContact::check(second, &verifier));

        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
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
            book.get_validator_peer_ids(signer.validator_address()).len(),
            1
        );

        book.house_keeping();

        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
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
            &NoopValidatorRecordVerifier,
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
    fn own_contact_carries_a_verifiable_validator_record() {
        let (own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let validator_address = signer.validator_address().clone();

        book.set_validator_record_signer(Some(signer), &own_key);

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

        // Refreshing the contact re-signs the record for the new timestamp.
        book.update_own_contact(&own_key);
        let refreshed = book.get_own_contact().signed().clone();
        assert_eq!(
            refreshed.check_validator_claim(&verifier),
            Some(ValidatorVerification::Verified)
        );

        book.set_validator_record_signer(None, &own_key);
        assert!(book
            .get_own_contact()
            .signed()
            .inner
            .validator_info()
            .is_none());
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
    fn budget_exhaustion_does_not_evict_an_already_verified_validator() {
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
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );

        // The same peer refreshes its contact (newer timestamp, same claim), but this time the
        // shared budget is exhausted by contention from other connections.
        assert!(!budget.try_consume(), "budget should already be empty");
        let refreshed = contact_for(&peer_key, now, Some(&signer));
        book.insert(CheckedPeerContact::check_with_budget(
            refreshed, &verifier, &budget,
        ));

        // The validator must still be indexed: the refresh was never actually checked, so it
        // must not be treated as if the claim had failed.
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert!(book.get(&peer_id).unwrap().is_gossipable());
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

        // With nothing to carry forward from, the claim stays unverified (safe default), not
        // silently trusted.
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
            first, &verifier_a, &budget,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer_a.validator_address()),
            vec![peer_id]
        );

        // The same peer now claims a *different* validator address while the budget is
        // exhausted. Trust in the old address must not transfer to the new one.
        assert!(!budget.try_consume());
        let switched = contact_for(&peer_key, now, Some(&signer_b));
        book.insert(CheckedPeerContact::check_with_budget(
            switched, &verifier_a, &budget,
        ));

        assert!(book
            .get_validator_peer_ids(signer_a.validator_address())
            .is_empty());
        assert!(book
            .get_validator_peer_ids(signer_b.validator_address())
            .is_empty());
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
        budget.reset(1);
        let checked = CheckedPeerContact::check_with_budget(with_claim.clone(), &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Verified);

        // The budget is now exhausted, so a second claim in the same tick is left pending.
        let checked = CheckedPeerContact::check_with_budget(with_claim, &verifier, &budget);
        assert_eq!(checked.claim, ClaimCheck::Pending);
    }

    #[test]
    fn a_definitive_recheck_revokes_a_carried_forward_claim() {
        let (_own_key, mut book) = empty_book();
        let (signer, verifier) = validator(1);
        let peer_key = Keypair::generate_ed25519();
        let now = now_secs();

        // First sighting: budget is available, the claim verifies and gets indexed.
        let budget = ValidatorClaimBudget::new(1);
        let first = contact_for(&peer_key, now - 20, Some(&signer));
        let peer_id = first.peer_id();
        book.insert(CheckedPeerContact::check_with_budget(
            first, &verifier, &budget,
        ));
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );

        // The same peer_id refreshes its contact (newer timestamp, still claiming the same
        // validator address) while the shared budget happens to be exhausted, so the claim is
        // only carried forward, not actually re-checked.
        assert!(!budget.try_consume(), "budget should already be empty");
        let refreshed = contact_for(&peer_key, now - 10, Some(&signer));
        book.insert(CheckedPeerContact::check_with_budget(
            refreshed, &verifier, &budget,
        ));
        // Carrying forward keeps it indexed for now...
        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        // ...but the carried-forward belief must not be allowed to stand in for a real check
        // forever: the contact must still be a candidate for the periodic re-check sweep.
        let snapshot = book.unverified_validator_contacts();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(*snapshot[0].peer_id(), peer_id);

        // The re-check sweep now checks the still-carried contact for real (e.g. because the
        // validator's signing key rotated away in the meantime) and finds it does not verify.
        let results: Vec<_> = book
            .unverified_validator_contacts()
            .iter()
            .map(|info| {
                (
                    *info.peer_id(),
                    info.contact().timestamp(),
                    ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
                )
            })
            .collect();
        book.apply_validator_verifications(results);

        // The revocation must take effect: a carried-forward claim that fails a real check is no
        // longer indexed.
        assert!(book
            .get_validator_peer_ids(signer.validator_address())
            .is_empty());
        assert!(!book.get(&peer_id).unwrap().is_validator_verified());
    }

    #[test]
    fn an_inconclusive_recheck_never_demotes_a_verified_claim() {
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
        // own staking-contract state is transiently incomplete — must not demote the claim.
        let refreshed = contact_for(&peer_key, now, Some(&signer));
        let checked_refreshed = CheckedPeerContact {
            contact: refreshed,
            claim: ClaimCheck::Pending,
        };
        book.insert(checked_refreshed);

        assert_eq!(
            book.get_validator_peer_ids(signer.validator_address()),
            vec![peer_id]
        );
        assert!(book.get(&peer_id).unwrap().is_validator_verified());
    }
}
