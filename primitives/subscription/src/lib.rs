#[macro_use]
extern crate beserial_derive;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;

use std::collections::HashSet;

use beserial::{Deserialize, Serialize};
use keys::Address;
use primitives::coin::Coin;
use transaction::Transaction;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Subscription {
    #[beserial(discriminant = 0)]
    None,
    #[beserial(discriminant = 1)]
    Any,
    #[beserial(discriminant = 2)]
    Addresses(#[beserial(len_type(u16))] HashSet<Address>),
    #[beserial(discriminant = 3)]
    MinFee(Coin), // Fee per byte
}

impl Default for Subscription {
    fn default() -> Self {
        Subscription::None
    }
}

impl Subscription {
    pub fn matches_block(&self) -> bool {
        match self {
            Subscription::None => false,
            _ => true,
        }
    }

    pub fn matches_transaction(&self, transaction: &Transaction) -> bool {
        match self {
            Subscription::None => false,
            Subscription::Any => true,
            Subscription::Addresses(addresses) => addresses.contains(&transaction.sender),
            Subscription::MinFee(min_fee) => {
                // TODO: Potential overflow for u64
                min_fee
                    .checked_mul(transaction.serialized_size() as u64)
                    .map(|block_fee| transaction.fee >= block_fee)
                    .unwrap_or(true)
            }
        }
    }
}
