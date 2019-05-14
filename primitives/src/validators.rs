use crate::policy::ACTIVE_VALIDATORS;

use beserial::{Deserialize, Serialize};

use nimiq_keys::Address;
use nimiq_bls::bls12_381::lazy::LazyPublicKey;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub public_key: LazyPublicKey,
    pub reward_address_opt: Option<Address>,
    pub staker_address: Address,
}

impl Slot {
    #[inline]
    pub fn reward_address(&self) -> &Address {
        if let Some(ref addr) = self.reward_address_opt {
            addr
        } else {
            &self.staker_address
        }
    }
}

pub type Slots = [Slot; ACTIVE_VALIDATORS as usize];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Validator {
    pub public_key: LazyPublicKey,
    pub slots: u16
}

pub type Validators = Vec<Validator>;
