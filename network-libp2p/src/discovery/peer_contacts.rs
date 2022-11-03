use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use bitflags::bitflags;
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

        /// The ZKP that proves the latest election block
        ///
        const CHAIN_PROOF = 1 << 2;

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
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
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

    /// Returns true if the services provided are interesting to me
    pub fn matches(&self, services: Services) -> bool {
        self.services().contains(services)
    }

    pub fn get_score(&self) -> f64 {
        self.meta.read().score
    }

    pub fn set_score(&self, score: f64) {
        self.meta.write().score = score;
    }
}

#[derive(Debug)]
pub struct PeerContactBook {
    own_peer_contact: PeerContactInfo,

    peer_contacts: HashMap<PeerId, Arc<PeerContactInfo>>,
}

impl PeerContactBook {
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
        let info = PeerContactInfo::from(contact);
        let peer_id = info.peer_id;

        debug!(%peer_id, "Adding peer contact");

        self.peer_contacts.insert(peer_id, Arc::new(info));
    }

    pub fn insert_filtered(&mut self, contact: SignedPeerContact, services_filter: Services) {
        let info = PeerContactInfo::from(contact);
        if info.matches(services_filter) {
            log::debug!(
                " Inserting peer_id: {} into my peer contacts, because is interesting to me",
                info.peer_id
            );
            let peer_id = info.peer_id;
            self.peer_contacts.insert(peer_id, Arc::new(info));
        }
    }

    pub fn insert_all<I: IntoIterator<Item = SignedPeerContact>>(&mut self, contacts: I) {
        for contact in contacts {
            self.insert(contact);
        }
    }

    pub fn insert_all_filtered<I: IntoIterator<Item = SignedPeerContact>>(
        &mut self,
        contacts: I,

        services_filter: Services,
    ) {
        for contact in contacts {
            self.insert_filtered(contact, services_filter)
        }
    }

    pub fn get(&self, peer_id: &PeerId) -> Option<Arc<PeerContactInfo>> {
        self.peer_contacts.get(peer_id).map(Arc::clone)
    }

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

    pub fn add_own_addresses<I: IntoIterator<Item = Multiaddr>>(&mut self, addresses: I) {
        debug!(
            addresses = ?addresses.into_iter().collect::<Vec<Multiaddr>>(),
            "Addresses observed for us",
        );
        // TODO: We could add these observed addresses to our advertised addresses (with restrictions).
    }

    pub fn update_own_contact(&mut self, keypair: &Keypair) {
        // Not really optimal to clone here, but *shrugs*
        let mut contact = self.own_peer_contact.contact.inner.clone();

        // Update timestamp
        contact.set_current_time();

        self.insert(contact.sign(keypair));
    }

    pub fn get_own_contact(&self) -> &PeerContactInfo {
        &self.own_peer_contact
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

        let raw = hex::decode(&hex_encoded).map_err(D::Error::custom)?;

        beserial::Deserialize::deserialize_from_vec(&raw).map_err(D::Error::custom)
    }
}
