use std::collections::HashSet;

use keys::{PublicKey, Signature, KeyPair};

use crate::address::PeerId;


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub addresses: HashSet<Multiaddr>,
    pub services: (), // TODO
    pub timestamp: u64,
    pub public_key: PublicKey,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedPeerInfo {
    pub peer_info: PeerInfo,
    pub signature: Signature,
}


impl PeerInfo {
    pub fn sign(self, key_pair: &KeyPair) -> SignedPeerInfo {
        let peer_info_data = self.serialize_to_vec()
            .expect("Serialization failed");

        let signature = key_pair.sign(&peer_info_data);

        SignedPeerInfo {
            peer_info: self,
            signature,
        }
    }

    pub fn peer_id(&self) -> PeerId {
        PeerId::from(&self.public_key)
    }
}

impl SignedPeerInfo {
    pub fn verify_signature(&self, public_key: &PublicKey) -> bool {
        let peer_info_data = self.peer_info.serialize_to_vec()
            .expect("Serialization failed");

        public_key.verify(&self.signature, &peer_info_data)
    }
}
