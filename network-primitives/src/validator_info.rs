use block_albatross::signed::{SignedMessage, PREFIX_VALIDATOR_INFO, Message};
use crate::address::peer_address::PeerAddress;
use bls::bls12_381::CompressedPublicKey;
use beserial::{Serialize, Deserialize};
use hex::FromHex;
use hash::{SerializeContent, Hash, Blake2bHash};


create_typed_array!(ValidatorId, u8, 16);
add_hex_io_fns_typed_arr!(ValidatorId, ValidatorId::SIZE);

impl ValidatorId {
    pub fn from_public_key(public_key: &CompressedPublicKey) -> Self {
        let mut id: [u8; 16] = [0; 16];
        id.copy_from_slice(&public_key.hash::<Blake2bHash>().as_bytes()[0..16]);
        ValidatorId(id)
    }
}


/// Information regarding an (maybe active) validator
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent)]
pub struct ValidatorInfo {
    /// The validator ID
    /// TODO: obsolete
    #[deprecated]
    pub validator_id: ValidatorId,

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
