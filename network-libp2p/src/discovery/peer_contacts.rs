use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use bitflags::bitflags;
use instant::SystemTime;
use libp2p::{
    gossipsub::Gossipsub,
    identity::{Keypair, PublicKey},
    Multiaddr, PeerId,
};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::{TaggedKeypair, TaggedSignable, TaggedSignature};

bitflags! {
    /// Bitmask of services
    ///
    ///
    ///  - This just serializes to its numeric value for serde, but a list of strings would be nicer.
    ///
    #[derive(Serialize, Deserialize)]
    #[cfg_attr(feature = "peer-contact-book-persistence", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
    pub struct Services: u32 {
        /// The node provides at least the latest [`nimiq_primitives::policy::NUM_BLOCKS_VERIFICATION`] as full blocks.
        ///
        const FULL_BLOCKS = 1 << 0;

        /// The node provides the full transaction history.
        ///
        const HISTORY = 1 << 1;

        /// The node provides inclusion and exclusion proofs for accounts that are necessary to verify active accounts as
        /// well as accounts in all transactions it provided from its mempool.
        ///
        /// However, if [`Services::ACCOUNTS_CHUNKS`] is not set, the node may occasionally not provide a proof if it
        /// decided to prune the account from local storage.
        ///
        const ACCOUNTS_PROOF = 1 << 3;

        /// The node provides the full accounts tree in form of chunks.
        /// This implies that the client stores the full accounts tree.
        ///
        const ACCOUNTS_CHUNKS = 1 << 4;

        /// The node tries to stay on sync with the network wide mempool and will provide access to it.
        ///
        /// Nodes that do not have this flag set may occasionally announce transactions from their mempool and/or reply to
        /// mempool requests to announce locally crafted transactions.
        ///
        const MEMPOOL = 1 << 5;

        /// The node provides an index of transactions allowing it to find historic transactions by address or by hash.
        ///  Only history nodes will have this flag set
        /// Nodes that have this flag set may prune any part of their transaction index at their discretion, they do not
        /// claim completeness of their results either.
        ///
        const TRANSACTION_INDEX = 1 << 6;

        /// This node accepts validator related messages.
        ///
        const VALIDATOR = 1 << 7;
    }
}

/// A plain peer contact. This contains:
///
///  - A set of multi-addresses for the peer.
///  - The peer's public key.
///  - A bitmask of the services supported by this peer.
///  - A timestamp when this contact information was generated.
///
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(
    feature = "peer-contact-book-persistence",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct PeerContact {
    #[beserial(len_type(u8))]
    pub addresses: Vec<Multiaddr>,

    /// Public key of this peer.
    #[cfg_attr(
        feature = "peer-contact-book-persistence",
        serde(with = "self::serde_public_key")
    )]
    pub public_key: PublicKey,

    /// Services supported by this peer.
    pub services: Services,

    /// Timestamp when this peer contact was created in *seconds* since unix epoch. `None` if this is a seed.
    pub timestamp: Option<u64>,
}

impl PeerContact {
    pub fn new<I: IntoIterator<Item = Multiaddr>>(
        addresses: I,
        public_key: PublicKey,
        services: Services,
        timestamp: Option<u64>,
    ) -> Self {
        let mut addresses = addresses.into_iter().collect::<Vec<Multiaddr>>();

        addresses.sort();

        Self {
            addresses,
            public_key,
            services,
            timestamp,
        }
    }

    /// Returns whether this is a seed peer contact. See [`PeerContact::timestamp`].
    pub fn is_seed(&self) -> bool {
        self.timestamp.is_none()
    }

    /// Derives the peer ID from the public key
    pub fn peer_id(&self) -> PeerId {
        self.public_key.clone().to_peer_id()
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
        self.timestamp = Some(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
    }

    /// Adds a set of addresses
    pub fn add_addresses(&mut self, addresses: Vec<Multiaddr>) {
        self.addresses.extend(addresses)
    }

    /// Removes addresses
    pub fn remove_addresses(&mut self, addresses: Vec<Multiaddr>) {
        let to_remove_addresses: HashSet<Multiaddr> = HashSet::from_iter(addresses);
        self.addresses
            .retain(|addr| !to_remove_addresses.contains(addr));
    }
}

impl TaggedSignable for PeerContact {
    const TAG: u8 = 0x02;
}

/// A signed peer contact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(
    feature = "peer-contact-book-persistence",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct SignedPeerContact {
    /// The wrapped peer contact.
    #[cfg_attr(feature = "peer-contact-book-persistence", serde(flatten))]
    pub inner: PeerContact,

    /// The signature over the serialized peer contact.
    pub signature: TaggedSignature<PeerContact, Keypair>,
}

impl SignedPeerContact {
    /// Verifies that the signature is valid for this peer contact.
    pub fn verify(&self) -> bool {
        self.signature
            .tagged_verify(&self.inner, &self.inner.public_key)
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.inner.public_key
    }
}

/// Meta information attached to peer contact info objects. This is meant to be mutable and change over time.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "peer-contact-book-persistence",
    derive(serde::Serialize, serde::Deserialize)
)]
struct PeerContactMeta {
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
            meta: RwLock::new(PeerContactMeta { score: 0. }),
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

    /// Returns an iterator over the multi-addresses of this contact.
    pub fn addresses(&self) -> impl Iterator<Item = &Multiaddr> {
        self.contact.inner.addresses.iter()
    }

    /// Returns whether this is a seed contact.
    pub fn is_seed(&self) -> bool {
        self.contact.inner.timestamp.is_none()
    }
    /// Returns whether the peer contact exceeds its age limit
    pub fn exceeds_age(&self, max_age: Duration, unix_time: Duration) -> bool {
        if let Some(timestamp) = self.contact.inner.timestamp {
            if let Some(age) = unix_time.checked_sub(Duration::from_secs(timestamp)) {
                return age > max_age;
            }
        }
        false
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
}

/// Main structure that holds the peer information that has been obtained or
/// discovered by the discovery protocol.
#[derive(Debug)]
pub struct PeerContactBook {
    /// Contact information for our own.
    own_peer_contact: PeerContactInfo,
    /// Contact information for other peers in the network indexed by their
    /// peer ID.
    peer_contacts: HashMap<PeerId, Arc<PeerContactInfo>>,
}

impl PeerContactBook {
    /// If a peer's age exceeds this value in seconds, it is removed (30 minutes)
    pub const MAX_PEER_AGE: u64 = 30 * 60;

    /// Creates a new `PeerContactBook` given our own peer contact information.
    pub fn new(own_peer_contact: SignedPeerContact) -> Self {
        Self {
            own_peer_contact: own_peer_contact.into(),
            peer_contacts: HashMap::new(),
        }
    }

    /// Insert a peer contact or update an existing one
    ///
    /// # TODO
    ///
    ///  - Check if the peer is already known and update its information.
    ///
    pub fn insert(&mut self, contact: SignedPeerContact) {
        log::debug!(peer_id = %contact.inner.peer_id(), addresses = ?contact.inner.addresses, "Adding peer contact");

        let info = PeerContactInfo::from(contact);
        let peer_id = info.peer_id;

        self.peer_contacts.insert(peer_id, Arc::new(info));
    }

    /// Inserts a peer contact or update an existing using the service filtering.
    /// If the filter matches the services provided by the contact, it is added.
    /// Otherwise it is ignored.
    pub fn insert_filtered(&mut self, contact: SignedPeerContact, services_filter: Services) {
        let info = PeerContactInfo::from(contact);
        if info.matches(services_filter) {
            log::trace!(
                added_peer = %info.peer_id,
                services = ?info.services(),
                addresses = ?info.contact.inner.addresses,
                "Inserting into my peer contacts, because is interesting to me",
            );

            let peer_id = info.peer_id;
            self.peer_contacts.insert(peer_id, Arc::new(info));
        }
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
    ) {
        for contact in contacts {
            self.insert_filtered(contact, services_filter)
        }
    }

    /// Gets a peer contact if it exists given its peer_id.
    /// If the peer_id is not found, `None` is returned.
    pub fn get(&self, peer_id: &PeerId) -> Option<Arc<PeerContactInfo>> {
        self.peer_contacts.get(peer_id).map(Arc::clone)
    }

    /// Gets a set of peer contacts given a services filter.
    /// Every peer contact that matches such services will be returned.
    pub fn query(&self, services: Services) -> impl Iterator<Item = Arc<PeerContactInfo>> + '_ {
        // TODO: This is a naive implementation
        // TODO: Sort by score?
        self.peer_contacts.iter().filter_map(move |(_, contact)| {
            if !contact.is_seed() && contact.matches(services) {
                Some(Arc::clone(contact))
            } else {
                None
            }
        })
    }

    /// Updates the score of every peer in the contact book with the gossipsub
    /// peer score.
    pub fn update_scores(&self, gossipsub: &Gossipsub) {
        let contacts = self.peer_contacts.iter();

        for contact in contacts {
            if let Some(score) = gossipsub.peer_score(contact.0) {
                contact.1.set_score(score);
            } else {
                debug!(peer_id = %contact.0, "No score for peer");
            }
        }
    }

    /// Adds a set of addresses to the list of addresses known for our own.
    pub fn add_own_addresses<I: IntoIterator<Item = Multiaddr>>(
        &mut self,
        addresses: I,
        keypair: &Keypair,
    ) {
        let mut contact = self.own_peer_contact.contact.inner.clone();
        let addresses = addresses.into_iter().collect::<Vec<Multiaddr>>();
        debug!(?addresses, "Adding addresses observed for our own");
        contact.add_addresses(addresses);
        self.insert(contact.sign(keypair));
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
        self.insert(contact.sign(keypair));
    }

    /// Updates the timestamp our own contact
    pub fn update_own_contact(&mut self, keypair: &Keypair) {
        // Not really optimal to clone here, but *shrugs*
        let mut contact = self.own_peer_contact.contact.inner.clone();

        // Update timestamp
        contact.set_current_time();

        self.insert(contact.sign(keypair));
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
}

#[cfg(feature = "peer-contact-book-persistence")]
mod serde_public_key {
    use libp2p::identity::PublicKey;
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(public_key: &PublicKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_encoded = hex::encode(beserial::Serialize::serialize_to_vec(public_key));

        Serialize::serialize(&hex_encoded, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PublicKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_encoded: String = Deserialize::deserialize(deserializer)?;

        let raw = hex::decode(hex_encoded).map_err(D::Error::custom)?;

        beserial::Deserialize::deserialize_from_vec(&raw).map_err(D::Error::custom)
    }
}
