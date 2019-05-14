extern crate nimiq_bls as bls;
extern crate nimiq_keys as keys;
use crate::policy::ACTIVE_VALIDATORS;

use beserial::{Deserialize, Serialize};

use keys::Address;
use bls::bls12_381::PublicKey;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub public_key: PublicKey,
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
    pub public_key: PublicKey,
    pub slots: u16
}

pub type Validators = Vec<Validator>;
