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
use nimiq_network_interface::peer_info::Services;
use nimiq_utils::tagged_signing::{TaggedKeypair, TaggedSignable, TaggedSignature};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// A plain peer contact. This contains:
///
///  - A set of multi-addresses for the peer.
///  - The peer's public key.
///  - A bitmask of the services supported by this peer.
///  - A timestamp when this contact information was generated.
///
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerContact {
    pub addresses: Vec<Multiaddr>,

    /// Public key of this peer.
    #[serde(with = "self::serde_public_key")]
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
pub struct SignedPeerContact {
    /// The wrapped peer contact.
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

    /// Gets the public key of this peer contact.
    pub fn public_key(&self) -> &PublicKey {
        &self.inner.public_key
    }

    /// Gets the Peer ID that results from this peer contact's peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }
}

/// Meta information attached to peer contact info objects. This is meant to be mutable and change over time.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// Own Peer ID (also present in `own_peer_contact`)
    own_peer_id: PeerId,
    /// Contact information for other peers in the network indexed by their
    /// peer ID.
    peer_contacts: HashMap<PeerId, Arc<PeerContactInfo>>,
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
            only_secure_addresses,
            allow_loopback_addresses,
            memory_transport,
        }
    }

    /// Insert a peer contact or update an existing one
    pub fn insert(&mut self, contact: SignedPeerContact) {
        // Don't insert our own contact into our peer contacts
        if contact.peer_id() == self.own_peer_id {
            return;
        }

        log::debug!(peer_id = %contact.peer_id(), addresses = ?contact.inner.addresses, "Adding peer contact");
        let current_ts = Some(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );

        let info = PeerContactInfo::from(contact);
        let peer_id = info.peer_id;

        match self.peer_contacts.entry(peer_id) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let entry_value = entry.get_mut();
                // Only update the contact if the timestamp is greater than the entry we have for this peer
                // and if the timestamp of the peer contact is not in the future
                if entry_value.contact().timestamp < info.contact().timestamp
                    && info.contact().timestamp <= current_ts
                {
                    *entry_value = Arc::new(info);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Arc::new(info));
            }
        }
    }

    /// Inserts a peer contact or update an existing using the service filtering.
    /// If the filter matches the services provided by the contact, it is added.
    /// Otherwise it is ignored.
    /// The services_filter argument to this function contains the services that are required.
    pub fn insert_filtered(
        &mut self,
        contact: SignedPeerContact,
        services_filter: Services,
        only_secure_ws_connections: bool,
    ) {
        let info = PeerContactInfo::from(contact);

        // A peer is interesting to us in two cases:
        // - I'm configured as a validator, and the peer is also a validator, then that peer is interesting to me
        //   regardless of the services that are provided by that peer.
        // - The services provided by the peer are a superset of the requested services.
        if (self
            .own_peer_contact
            .services()
            .contains(Services::VALIDATOR)
            && info.services().contains(Services::VALIDATOR))
            || info.matches(services_filter)
        {
            let has_secure_ws_connections = info
                .contact
                .inner
                .addresses
                .iter()
                .any(Self::is_address_ws_secure);

            if !only_secure_ws_connections || has_secure_ws_connections {
                log::trace!(
                    added_peer = %info.peer_id,
                    services = ?info.services(),
                    addresses = ?info.contact.inner.addresses,
                    only_secure_ws_connections,
                    "Inserting into my peer contacts, because the peer is also a validator or because it is interesting to us",
                );
                let peer_id = info.peer_id;
                self.peer_contacts.insert(peer_id, Arc::new(info));
            }
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
        only_secure_ws_connections: bool,
    ) {
        for contact in contacts {
            self.insert_filtered(contact, services_filter, only_secure_ws_connections)
        }
    }

    /// Gets a peer contact if it exists given its peer_id.
    /// If the peer_id is not found, `None` is returned.
    pub fn get(&self, peer_id: &PeerId) -> Option<Arc<PeerContactInfo>> {
        self.peer_contacts.get(peer_id).map(Arc::clone)
    }

    /// Gets the peer contact's addresses if it exists given its peer_id.
    /// If the peer_id is not found, `None` is returned.
    pub fn get_addresses(&self, peer_id: &PeerId) -> Option<Vec<Multiaddr>> {
        self.peer_contacts.get(peer_id).map(|e| {
            e.contact()
                .addresses
                .iter()
                .filter(|&address| self.is_address_dialable(address))
                .cloned()
                .collect()
        })
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

    /// Adds a set of addresses to the list of addresses known for our own.
    pub fn add_own_addresses<I: IntoIterator<Item = Multiaddr>>(
        &mut self,
        addresses: I,
        keypair: &Keypair,
    ) {
        let mut contact = self.own_peer_contact.contact.inner.clone();
        let addresses = addresses.into_iter().collect::<Vec<Multiaddr>>();
        trace!(?addresses, "Adding addresses observed for us");
        contact.add_addresses(addresses);
        self.own_peer_contact = PeerContactInfo::from(contact.sign(keypair));
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
        self.own_peer_contact = PeerContactInfo::from(contact.sign(keypair));
    }

    /// Updates the timestamp of our own contact
    pub fn update_own_contact(&mut self, keypair: &Keypair) {
        // Not really optimal to clone here, but *shrugs*
        let mut contact = self.own_peer_contact.contact.inner.clone();

        // Update timestamp
        contact.set_current_time();

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

    /// Returns true if an address is a secure websocket connection.
    /// If address doesn't have the websocket protocol, it will return `false`.
    fn is_address_ws_secure(address: &Multiaddr) -> bool {
        address.into_iter().any(|p| matches!(p, Protocol::Wss(_)))
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
