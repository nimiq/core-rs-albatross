use block_albastross::signed::{SignedMessage, PREFIX_VALIDATOR_INFO, Message};
use crate::address::peer_address::PeerAddress;
use bls::bls12_381::PublicKey;
use beserial::{Serialize, Deserialize};
use hex::FromHex;
use hash::SerializeContent;


create_typed_array!(ValidatorId, u8, 16);
add_hex_io_fns_typed_arr!(ValidatorId, ValidatorId::SIZE);

/// Information regarding an (maybe active) validator
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent)]
pub struct ValidatorInfo {
    /// The validator ID
    pub validator_id: ValidatorId,

    /// The validator's public key (BLS12-381)
    pub public_key: PublicKey,

    /// The validator's peer address
    pub peer_address: PeerAddress,

    /// If the validator is active, this is it's index in the active validator list
    pub pk_idx: Option<u16>,
}

impl Message for ValidatorInfo {
    const PREFIX: u8 = PREFIX_VALIDATOR_INFO;
}

/// The signed version of a ValidatorInfo
pub type SignedValidatorInfo = SignedMessage<ValidatorInfo>;
