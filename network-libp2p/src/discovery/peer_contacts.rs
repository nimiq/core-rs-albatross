use std::{
    collections::{HashMap, HashSet},
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
        let peer_id = contact.inner.peer_id();

        Self {
            peer_id,
            contact,
            meta: RwLock::new(PeerContactMeta {
                score: 0.,
                outer_protocol_address: None,
            }),
        }
    }
}

impl PeerContactInfo {
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
    pub fn insert(&mut self, contact: SignedPeerContact) {
        // Don't insert our own contact into our peer contacts
        if contact.peer_id() == self.own_peer_id {
            return;
        }

        log::debug!(peer_id = %contact.peer_id(), addresses = ?contact.inner.addresses, "Adding peer contact");
        let current_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Reject contacts with timestamps in the future
        if contact.inner.timestamp > current_ts {
            return;
        }

        self.store(contact);
    }

    /// Stores a contact, replacing an existing one only if the new one is strictly newer.
    fn store(&mut self, contact: SignedPeerContact) {
        let peer_id = contact.inner.peer_id();

        if let Some(existing) = self.peer_contacts.get(&peer_id) {
            // Only update the contact if the timestamp is greater than the entry we have
            if existing.contact().timestamp >= contact.inner.timestamp {
                return;
            }
        } else {
            log::trace!(
                peer_id = %peer_id,
                services = ?contact.inner.services,
                addresses = ?contact.inner.addresses,
                validator_address = ?contact.inner.validator_address(),
                "Adding peer contact",
            );
        }

        self.peer_contacts
            .insert(peer_id, Arc::new(PeerContactInfo::from(contact)));
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
        contact: SignedPeerContact,
        services_filter: Services,
        only_secure_ws_connections: bool,
    ) {
        // Don't insert our own contact into our peer contacts. Peers do echo it back to us.
        if contact.peer_id() == self.own_peer_id {
            return;
        }

        let filter = InsertFilter {
            services: services_filter,
            only_secure_ws_connections,
        };
        if !self.passes_filter(&contact.inner, &filter) {
            return;
        }

        self.store(contact);
    }

    /// Inserts a set of contacts or updates existing ones
    pub fn insert_all<I: IntoIterator<Item = SignedPeerContact>>(&mut self, contacts: I) {
        for contact in contacts {
            self.insert(contact);
        }
    }

    /// Inserts a set of peer contact or update an existing ones using the service
    /// filtering. If the filter matches the services provided by the contact,
    /// it is added. Otherwise it is ignored.
    pub fn insert_all_filtered<I: IntoIterator<Item = SignedPeerContact>>(
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
                self.peer_contacts.remove(&peer_id);
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
    use nimiq_network_interface::validator_record::ValidatorRecord;
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;

    use super::*;

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
}
