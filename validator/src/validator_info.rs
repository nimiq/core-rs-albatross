use std::net::SocketAddr;

use beserial::{Deserialize, Serialize};
use block_albatross::signed::{Message, SignedMessage, PREFIX_VALIDATOR_INFO};
use bls::CompressedPublicKey;
use hash::SerializeContent;
use hash_derive::SerializeContent;
use network_interface::message::Message as NetworkMessage;
use peer_address::address::PeerAddress;

/// Information regarding an (maybe active) validator
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, Eq)]
pub struct ValidatorInfo {
    /// The validator's public key (BLS12-381)
    pub public_key: CompressedPublicKey,

    /// The validator's peer address
    pub peer_address: PeerAddress,

    /// An optional UDP address for faster communication
    pub udp_address: Option<SocketAddr>,

    /// From which block number this validator info is valid. It can be used as valid before, but
    /// we'll only accept validator info's that are newer that the newest we already know.
    pub valid_from: u32,
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

/// A list of signed ValidatorInfos which is sent across the network
#[derive(Deserialize, Serialize)]
pub struct SignedValidatorInfos(#[beserial(len_type(u16))] pub Vec<SignedValidatorInfo>);

impl NetworkMessage for SignedValidatorInfos {
    const TYPE_ID: u64 = 111;
}

impl Into<Vec<SignedValidatorInfo>> for SignedValidatorInfos {
    fn into(self) -> Vec<SignedValidatorInfo> {
        self.0
    }
}
