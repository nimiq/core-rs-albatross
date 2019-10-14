extern crate nimiq_bls as bls;
extern crate nimiq_keys as keys;

use beserial::{Deserialize, Serialize};
use bls::bls12_381::lazy::LazyPublicKey;
use keys::Address;
use nimiq_collections::grouped_list::{Group, GroupedList};

use crate::coin::Coin;
use crate::policy::TWO_THIRD_SLOTS;

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IndexedSlot {
    // Index in honest slots
    pub idx: u16,
    pub slot: Slot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Slots {
    #[beserial(len_type(u16))]
    slots: Vec<Slot>,
    slash_fine: Coin,
}

impl Slots {
    pub fn new(slots: Vec<Slot>, slash_fine: Coin) -> Self {
        Self {
            slots,
            slash_fine,
        }
    }

    pub fn slash_fine(&self) -> Coin {
        self.slash_fine
    }

    pub fn enough_votes(&self, num_votes: u16) -> bool {
        num_votes > TWO_THIRD_SLOTS
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn get(&self, index: usize) -> &Slot {
        &self.slots[index]
    }

    /*pub fn remove(&mut self, index: usize) -> Slot {
        self.slots.remove(index)
    }*/

    pub fn iter(&self) -> std::slice::Iter<Slot> {
        self.slots.iter()
    }
}

impl IntoIterator for Slots {
    type Item = Slot;
    type IntoIter = ::std::vec::IntoIter<Slot>;

    fn into_iter(self) -> Self::IntoIter {
        self.slots.into_iter()
    }
}

pub type Validator = Group<LazyPublicKey>;
pub type Validators = GroupedList<LazyPublicKey>;

impl From<Slots> for Validators {
    fn from(slots: Slots) -> Self {
        slots.iter()
            .map(|slot| slot.public_key.clone())
            .collect()
    }
}
