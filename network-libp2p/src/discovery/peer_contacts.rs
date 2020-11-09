use std::{
    collections::{HashSet, HashMap},
    sync::Arc,
    time::{SystemTime, Duration, UNIX_EPOCH},
};

use libp2p::{
    core::multiaddr::Protocol,
    identity::{Keypair, PublicKey},
    Multiaddr, PeerId,
};
use bitflags::bitflags;
use parking_lot::RwLock;

use beserial::{Serialize, Deserialize};

use crate::tagged_signing::{TaggedSignature, TaggedSignable, TaggedPublicKey, TaggedKeypair};


/// Configuration for the peer contact book.
#[derive(Clone, Debug)]
pub struct PeerContactBookConfig {
    pub max_age_websocket: Duration,
    pub max_age_webrtc: Duration,
    pub max_age_dumb: Duration,
}

impl Default for PeerContactBookConfig {
    fn default() -> Self {
        Self {
            max_age_websocket: Duration::from_secs(60 * 30),
            max_age_webrtc: Duration::from_secs(60 * 15),
            max_age_dumb: Duration::from_secs(60),
        }
    }
}

impl PeerContactBookConfig {
    /// Returns the max age for this protocol
    pub fn protocols_max_age(&self, protocols: Protocols) -> Duration {
        if protocols.contains(Protocols::WS) || protocols.contains(Protocols::WSS) {
            self.max_age_websocket
        }
        else if protocols.contains(Protocols::RTC) {
            self.max_age_webrtc
        }
        else {
            self.max_age_dumb
        }
    }
}


bitflags! {
    /// Bitmask of services
    ///
    /// # TODO
    ///
    ///  - This just serializes to its numeric value for serde, but a list of strings would be nicer.
    ///
    #[derive(Serialize, Deserialize)]
    #[cfg_attr(feature = "peer-contact-book-persistence", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
    pub struct Services: u32 {
        /// The node provides at least the latest [`nimiq_primitives::policy::NUM_BLOCKS_VERIFICATION`] as full blocks.
        ///
        const FULL_BLOCKS = 1 << 0;

        /// The node provides the full block history.
        ///
        /// If {@link Services.FULL_BLOCKS} is set, these blocks are provided as full blocks.
        ///
        const BLOCK_HISTORY = 1 << 1;

        /// The node provides a proof that a certain block is included in the current chain.
        ///
        /// If [[`Services::FULL_BLOCKS`] is set, these blocks may be requested as full blocks.
        ///
        /// However, if [`Services::BLOCK_HISTORY`] is not set, this service is only provided for the latest
        /// [`nimiq_primitives::policy::NUM_BLOCKS_VERIFICATION`] blocks.
        ///
        const BLOCK_PROOF = 1 << 2;

        /// The node provides a chain proof for the tip of the current main chain.
        ///
        const CHAIN_PROOF = 1 << 3;

        /// The node provides inclusion and exclusion proofs for accounts that are necessary to verify active accounts as
        /// well as accounts in all transactions it provided from its mempool.
        ///
        /// However, if [`Services::ACCOUNTS_CHUNKS`] is not set, the node may occasionally not provide a proof if it
        /// decided to prune the account from local storage.
        ///
        const ACCOUNTS_PROOF = 1 << 4;

        /// The node provides the full accounts tree in form of chunks.
        /// This implies that the client stores the full accounts tree.
        ///
        const ACCOUNTS_CHUNKS = 1 << 5;

        /// The node tries to stay on sync with the network wide mempool and will provide access to it.
        ///
        /// Nodes that do not have this flag set may occasionally announce transactions from their mempool and/or reply to
        /// mempool requests to announce locally crafted transactions.
        ///
        const MEMPOOL = 1 << 6;

        /// The node provides an index of transactions allowing it to find historic transactions by address or by hash.
        ///
        /// Nodes that have this flag set may prune any part of their transaction index at their discretion, they do not
        /// claim completeness of their results either.
        ///
        const TRANSACTION_INDEX = 1 << 7;

        /// The node provides proofs for details from the block body, i.e. transaction proofs.
        ///
        /// However, if {@link Services.BLOCK_HISTORY} is not set, this service is only provided for the latest
        /// [`nimiq_primitives::policy::NUM_BLOCKS_VERIFICATION`] blocks.
        ///
        const BODY_PROOF = 1 << 8;

        /// This node accepts validator related messages.
        ///
        const VALIDATOR = 1 << 9;
    }
}

bitflags! {
    /// Bitmask of protocols
    ///
    /// # TODO
    ///
    ///  - This just serializes to its numeric value for serde, but a list of strings would be nicer.
    ///
    #[derive(Serialize, Deserialize)]
    #[cfg_attr(feature = "peer-contact-book-persistence", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
    pub struct Protocols: u32 {
        /// WebSocket (insecure)
        const WS = 1 << 0;

        /// WebSocket (secure)
        const WSS = 1 << 1;

        /// WebRTC
        const RTC = 1 << 2;

        /// Memory transport (for testing)
        #[cfg(test)]
        const MEM = 1 << 31;
    }
}

impl Protocols {
    // TODO: Put into a `PeerDiscoveryConfig`
    pub const MAX_AGE_WEBSOCKET: Duration = Duration::from_secs(60 * 30); // 30 minutes
    pub const MAX_AGE_WEBRTC: Duration = Duration::from_secs(60 * 15); // 15 minutes
    pub const MAX_AGE_DUMB: Duration = Duration::from_secs(60); // 1 minute

    pub fn from_multiaddrs<'a>(addresses: impl Iterator<Item = &'a Multiaddr>) -> Self {
        let mut protocols = Protocols::empty();
        for addr in addresses {
            protocols |= Protocols::from_multiaddr(addr);
        }
        protocols
    }

    pub fn from_multiaddr(multiaddr: &Multiaddr) -> Self {
        let protocol = multiaddr.iter().last()
            .unwrap_or_else(|| panic!("Empty multiaddr: {}", multiaddr));

        match protocol {
            Protocol::Ws(_) => Self::WS,
            Protocol::Wss(_) => Self::WSS,
            #[cfg(test)]
            Protocol::Memory(_) => Self::MEM,
            _ => Self::empty(),
        }
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
#[cfg_attr(feature = "peer-contact-book-persistence", derive(serde::Serialize, serde::Deserialize))]
pub struct PeerContact {
    #[beserial(len_type(u8))]
    pub addresses: HashSet<Multiaddr>,

    /// Public key of this peer.
    #[cfg_attr(feature = "peer-contact-book-persistence", serde(with = "self::serde_public_key"))]
    pub public_key: PublicKey,

    /// Services supported by this peer.
    pub services: Services,

    /// Timestamp when this peer contact was created in milliseconds since unix epoch. `None` if this is a seed.
    pub timestamp: Option<u64>,
}

impl PeerContact {
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
            signature
        }
    }

    /// Returns whether this is a seed peer contact. See [`PeerContact::timestamp`].
    pub fn is_seed(&self) -> bool {
        self.timestamp.is_none()
    }

    /// This sets the timestamp in the peer contact to the current system time.
    pub fn set_current_time(&mut self) {
        self.timestamp = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
    }
}

impl TaggedSignable for PeerContact {
    const TAG: u8 = 0x02;
}

/// A signed peer contact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "peer-contact-book-persistence", derive(serde::Serialize, serde::Deserialize))]
pub struct SignedPeerContact {
    /// The wrapped peer contact.
    #[cfg_attr(feature = "peer-contact-book-persistence", serde(flatten))]
    pub inner: PeerContact,

    /// The signature over the serialized peer contact.
    pub signature: TaggedSignature<PeerContact>,
}

impl SignedPeerContact {
    /// Verifies that the signature is valid for this peer contact.
    pub fn verify(&self) -> bool {
        self.inner.public_key.tagged_verify(&self.inner, &self.signature)
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.inner.public_key
    }
}


/// Meta information attached to peer contact info objects. This are meant to be mutable and change over time.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "peer-contact-book-persistence", derive(serde::Serialize, serde::Deserialize))]
struct PeerContactMeta {
    score: f32,

    /// The peers that sent us this contact. Used for scoring.
    ///
    /// # TODO
    ///
    ///  - Somehow serialize this
    ///
    #[cfg_attr(feature = "peer-contact-book-persistence", serde(skip))]
    reported_by: HashSet<PeerId>,
}


/// This encapsulates a peer contact (signed), but also pre-computes frequently used values such as `peer_id` and
/// `protocols`. It also contains meta-data that can be mutated.
#[derive(Debug)]
pub struct PeerContactInfo {
    /// The peer ID derived from the public key in the peer contact.
    peer_id: PeerId,

    /// The peer contact data with signature.
    contact: SignedPeerContact,

    /// Supported protocols extracted from multi addresses in the peer contact.
    protocols: Protocols,

    /// Mutable meta-data.
    meta: RwLock<PeerContactMeta>,
}

impl From<SignedPeerContact> for PeerContactInfo {
    fn from(contact: SignedPeerContact) -> Self {
        let peer_id = contact.inner.public_key.clone().into_peer_id();
        let protocols = Protocols::from_multiaddrs(contact.inner.addresses.iter());

        Self {
            peer_id,
            contact,
            protocols,
            meta: RwLock::new(PeerContactMeta {
                score: 0.,

                reported_by: HashSet::new(),
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

    /// Returns the supported protocols of this contact.
    pub fn protocols(&self) -> Protocols {
        self.protocols
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
    pub fn addresses<'a>(&'a self) -> impl Iterator<Item=&Multiaddr> + 'a {
        self.contact.inner.addresses.iter()
    }

    /// Returns whether this is a seed contact.
    pub fn is_seed(&self) -> bool {
        self.contact.inner.timestamp.is_none()
    }

    /// Returns whether the peer contact exceeds its age limit (specified in `config`).
    pub fn exceeds_age(&self, config: &PeerContactBookConfig) -> bool {
        if let Some(timestamp) = self.contact.inner.timestamp {
            if let Ok(unix_time) = SystemTime::now().duration_since(UNIX_EPOCH) {
                if let Some(age) = unix_time.checked_sub(Duration::from_millis(timestamp)) {
                    return age < config.protocols_max_age(self.protocols);
                }
            }
        }
        false
    }

    pub fn is_globally_reachable(&self) -> bool {
        for multiaddr in &self.contact.inner.addresses {
            for protocol in multiaddr.iter() {
                match protocol {
                    Protocol::Ip4(ip_addr) => return ip_addr.is_global(),
                    Protocol::Ip6(ip_addr) => return ip_addr.is_global(),
                    _ => {},
                }
            }
        }
        false
    }

    pub fn matches(&self, protocols: Protocols, services: Services) -> bool {
        self.protocols.intersects(protocols) && self.services().intersects(services)
    }
}

#[derive(Debug)]
pub struct PeerContactBook {
    config: PeerContactBookConfig,

    self_peer_contact: PeerContactInfo,

    peer_contacts: HashMap<PeerId, Arc<PeerContactInfo>>,
}

impl PeerContactBook {
    pub fn new(config: PeerContactBookConfig, self_peer_contact: SignedPeerContact) -> Self {
        Self {
            config,
            self_peer_contact: self_peer_contact.into(),
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
        log::debug!("Adding peer contact: {:?}", contact);

        let info = PeerContactInfo::from(contact);
        let peer_id = info.peer_id.clone();
        self.peer_contacts.insert(peer_id, Arc::new(info));
    }

    pub fn insert_filtered(&mut self, contact: SignedPeerContact, protocols_filter: Protocols, services_filter: Services) {
        let info = PeerContactInfo::from(contact);
        if info.matches(protocols_filter, services_filter) {
            let peer_id = info.peer_id.clone();
            self.peer_contacts.insert(peer_id, Arc::new(info));
        }
    }

    pub fn insert_all<I: IntoIterator<Item=SignedPeerContact>>(&mut self, contacts: I) {
        for contact in contacts {
            self.insert(contact);
        }
    }

    pub fn insert_all_filtered<I: IntoIterator<Item=SignedPeerContact>>(&mut self, contacts: I, protocols_filter: Protocols, services_filter: Services) {
        for contact in contacts {
            self.insert_filtered(contact, protocols_filter, services_filter)
        }
    }

    pub fn get(&self, peer_id: &PeerId) -> Option<Arc<PeerContactInfo>> {
        self.peer_contacts
            .get(peer_id)
            .map(|c| Arc::clone(c))
    }

    pub fn query<'a>(&'a self, protocols: Protocols, services: Services) -> impl Iterator<Item = Arc<PeerContactInfo>> + 'a {
        // TODO: This is a naive implementation
        // TODO: Sort by score?
        self.peer_contacts
            .iter()
            .filter_map(move |(_, contact)| {
                if !contact.is_seed() && contact.matches(protocols, services) {
                    Some(Arc::clone(contact))
                }
                else { None }
            })
    }

    pub fn self_add_addresses<I: IntoIterator<Item = Multiaddr>>(&mut self, addresses: I) {
        // TODO: We need to update our own `SignedPeerContact`, if we want to do so.
    }

    pub fn self_update(&mut self, keypair: &Keypair) {
        // Not really optimal to clone here, but *shrugs*
        let mut contact = self.self_peer_contact.contact.inner.clone();

        // Update timestamp
        contact.set_current_time();

        self.insert(contact.sign(keypair));
    }

    pub fn get_self(&self) -> &PeerContactInfo {
        &self.self_peer_contact
    }
}


#[cfg(test)]
mod tests {
    use super::Protocols;

    #[test]
    fn protocols_from_multiaddr() {
        assert_eq!(Protocols::from_multiaddr(&"/ip4/1.2.3.4/tcp/80/ws".parse().unwrap()), Protocols::WS);
        assert_eq!(Protocols::from_multiaddr(&"/ip4/1.2.3.4/tcp/443/wss".parse().unwrap()), Protocols::WSS);
    }

    #[test]
    fn protocols_from_multiaddrs() {
        assert_eq!(
            Protocols::from_multiaddrs(vec![
                "/ip4/1.2.3.4/tcp/80/ws".parse().unwrap(),
                "/dns/test.local/tcp/443/ws".parse().unwrap()
            ].iter()),
            Protocols::WS
        );

        assert_eq!(
            Protocols::from_multiaddrs(vec![
                "/ip4/1.2.3.4/tcp/443/ws".parse().unwrap(),
                "/ip4/1.2.3.4/tcp/443/wss".parse().unwrap()
            ].iter()),
            Protocols::WS | Protocols::WSS
        );
    }
}

#[cfg(feature = "peer-contact-book-persistence")]
mod serde_public_key {
    use libp2p::identity::PublicKey;

    use serde::{
        de::Error,
        Serialize, Serializer, Deserialize, Deserializer,
    };

    pub fn serialize<S>(public_key: &PublicKey, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let hex_encoded = hex::encode(beserial::Serialize::serialize_to_vec(public_key));

        Serialize::serialize(&hex_encoded, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PublicKey, D::Error>
        where D: Deserializer<'de>
    {
        let hex_encoded: String = Deserialize::deserialize(deserializer)?;

        let raw = hex::decode(&hex_encoded)
            .map_err(D::Error::custom)?;

        beserial::Deserialize::deserialize_from_vec(&raw)
            .map_err(D::Error::custom)
    }
}
