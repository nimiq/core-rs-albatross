use std::{
    collections::{HashSet, HashMap},
    sync::Arc,
    time::{SystemTime, Duration, UNIX_EPOCH},
};

use libp2p::{
    core::multiaddr::Protocol,
    Multiaddr,
};
use bitflags::bitflags;

use beserial::{Serialize, Deserialize};
use nimiq_keys::{PublicKey, Signature, KeyPair};
use nimiq_peer_address::address::PeerId;


// TODO: Should the `PeerAddressBook` live under the `DiscoveryProtocol`? I think so, because otherwise, the behaviour
// has to signal upwards everytime it receives a new peer address, without knowing if it already knows this one.
//
// TODO: Move Services, Protocols, PeerContact* into seperate module.

bitflags! {
    #[derive(Serialize, Deserialize)]
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
    #[derive(Serialize, Deserialize)]
    pub struct Protocols: u32 {
        /// WebSocket (insecure)
        const WS = 1 << 0;
        /// WebSocket (secure)
        const WSS = 1 << 1;
        /// WebRTC
        const RTC = 1 << 2;
    }
}

impl Protocols {
    // TODO: Put into a `PeerDiscoveryConfig`
    pub const MAX_AGE_WEBSOCKET: Duration = Duration::from_secs(60 * 30); // 30 minutes
    pub const MAX_AGE_WEBRTC: Duration = Duration::from_secs(60 * 15); // 15 minutes
    pub const MAX_AGE_DUMB: Duration = Duration::from_secs(60); // 1 minute


    pub fn from_multiaddr(multiaddr: &Multiaddr) -> Self {
        let protocol = multiaddr.iter().last()
            .unwrap_or_else(|| panic!("Empty multiaddr: {}", multiaddr));

        match protocol {
            Protocol::Ws(_) => Self::WS,
            Protocol::Wss(_) => Self::WSS,
            _ => Self::empty(),
        }
    }

    pub fn max_age(&self) -> Duration {
        if self.contains(Protocols::WS) || self.contains(Protocols::WSS) {
            Self::MAX_AGE_WEBSOCKET
        }
        else if self.contains(Protocols::RTC) {
            Self::MAX_AGE_WEBRTC
        }
        else {
            Self::MAX_AGE_DUMB
        }
    }
}


// TODO: Move to appropriate place
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerContact {
    #[beserial(len_type(u8))]
    pub addresses: HashSet<Multiaddr>,

    /// Public key of this peer.
    pub public_key: PublicKey,

    /// Services supported by this peer.
    pub services: Services,

    /// Timestamp when this peer contact was created in milliseconds since unix epoch. `None` if this is a seed.
    pub timestamp: Option<u64>,
}

impl PeerContact {
    pub fn peer_id(&self) -> PeerId {
        PeerId::from(&self.public_key)
    }

    pub fn sign(self, key_pair: &KeyPair) -> SignedPeerContact {
        let signature = key_pair.sign(&self.serialize_to_vec());
        SignedPeerContact {
            inner: self,
            signature
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedPeerContact {
    pub inner: PeerContact,
    pub signature: Signature,
}

impl SignedPeerContact {
    pub fn verify(&self) -> bool {
        self.inner.public_key.verify(&self.signature, &self.inner.serialize_to_vec())
    }
}


#[derive(Clone, Debug)]
pub struct PeerContactInfo {
    /// The peer ID derived from the public key in the peer contact.
    peer_id: PeerId,

    /// The peer contact data with signature.
    contact: SignedPeerContact,

    /// Supported protocols extracted from multi addresses in the peer contact.
    protocols: Protocols,

    // TODO
    score: f32,
}

impl From<SignedPeerContact> for PeerContactInfo {
    fn from(contact: SignedPeerContact) -> Self {
        let peer_id = contact.inner.peer_id();

        let mut protocols = Protocols::empty();
        for addr in &contact.inner.addresses {
            protocols |= Protocols::from_multiaddr(addr);
        }

        Self {
            peer_id,
            contact,
            protocols,
            score: 0.,
        }
    }
}

impl PeerContactInfo {
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn protocols(&self) -> Protocols {
        self.protocols
    }

    pub fn services(&self) -> Services {
        self.contact.inner.services
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.contact.inner.public_key
    }

    pub fn addresses<'a>(&'a self) -> impl Iterator<Item=&Multiaddr> + 'a {
        self.contact.inner.addresses.iter()
    }

    pub fn is_seed(&self) -> bool {
        self.contact.inner.timestamp.is_none()
    }

    pub fn exceeds_age(&self) -> bool {
        if let Some(timestamp) = self.contact.inner.timestamp {
            if let Ok(unix_time) = SystemTime::now().duration_since(UNIX_EPOCH) {
                if let Some(age) = unix_time.checked_sub(Duration::from_millis(timestamp)) {
                    return age < self.protocols.max_age();
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
}


#[derive(Clone, Debug, Default)]
pub struct PeerContactBook {
    peer_contacts: HashMap<PeerId, Arc<PeerContactInfo>>,
}

impl PeerContactBook {
    pub fn insert(&mut self, contact: SignedPeerContact) {
        let info = Arc::new(PeerContactInfo::from(contact));
        let peer_id = info.peer_id.clone();
        self.peer_contacts.insert(peer_id, info);
    }

    pub fn get(&self, peer_id: &PeerId) -> Option<Arc<PeerContactInfo>> {
        self.peer_contacts
            .get(peer_id)
            .map(|c| Arc::clone(c))
    }

    pub fn query<'a>(&'a self, protocols: Protocols, services: Services) -> impl Iterator<Item = Arc<PeerContactInfo>> + 'a {
        // TODO: This is a naive implementation
        self.peer_contacts
            .iter()
            .filter_map(move |(_, contact)| {
                if contact.protocols.contains(protocols) && contact.services().contains(services) {
                    Some(Arc::clone(contact))
                }
                else { None }
            })
    }
}


