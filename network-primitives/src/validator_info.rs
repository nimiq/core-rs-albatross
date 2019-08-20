use block_albatross::signed::{SignedMessage, PREFIX_VALIDATOR_INFO, Message};
use crate::address::peer_address::PeerAddress;
use bls::bls12_381::CompressedPublicKey;
use beserial::{Serialize, Deserialize};
use hex::FromHex;
use hash::{SerializeContent, Hash, Blake2bHash};



/// Information regarding an (maybe active) validator
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent)]
pub struct ValidatorInfo {
    /// The validator's public key (BLS12-381)
    pub public_key: CompressedPublicKey,

    /// The validator's peer address
    pub peer_address: PeerAddress,
}

impl PartialEq for ValidatorInfo {
    fn eq(&self, other: &ValidatorInfo) -> bool {
        self.public_key == other.public_key
    }
}

impl Message for ValidatorInfo {
    const PREFIX: u8 = PREFIX_VALIDATOR_INFO;
}

/// The signed version of a ValidatorInfo
pub type SignedValidatorInfo = SignedMessage<ValidatorInfo>;
