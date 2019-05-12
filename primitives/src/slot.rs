extern crate nimiq_bls as bls;
extern crate nimiq_keys as keys;

use beserial::{Deserialize, Serialize};

use keys::Address;
use bls::bls12_381::PublicKey;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub public_key: PublicKey,
    pub reward_address: Address,
    pub slashing_address: Address,
}
