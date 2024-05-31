use std::collections::HashSet;

use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::Transaction;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[repr(u8)]
pub enum Subscription {
    #[default]
    None,
    Any,
    Addresses(HashSet<Address>),
    MinFee(Coin), // Fee per byte
}

impl Subscription {
    pub fn matches_block(&self) -> bool {
        !matches!(self, Subscription::None)
    }

    pub fn matches_transaction(&self, transaction: &Transaction) -> bool {
        match self {
            Subscription::None => false,
            Subscription::Any => true,
            Subscription::Addresses(addresses) => addresses.contains(&transaction.sender),
            Subscription::MinFee(min_fee) => min_fee
                .checked_mul(transaction.serialized_size() as u64)
                .map(|block_fee| transaction.fee >= block_fee)
                .unwrap_or(true),
        }
    }
}
