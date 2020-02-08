extern crate nimiq_hash as hash;
extern crate nimiq_transaction as transaction;

use std::fmt::Debug;

use failure::Fail;

use beserial::{Deserialize, Serialize};
use hash::Blake2bHash;
use transaction::Transaction;

pub trait Block: Serialize + Deserialize {
    type Header: BlockHeader;
    type Error: BlockError;

    fn hash(&self) -> Blake2bHash;

    fn prev_hash(&self) -> &Blake2bHash;

    fn height(&self) -> u32;

    // TODO We should rather return a reference here.
    fn header(&self) -> Self::Header;

    fn transactions(&self) -> Option<&Vec<Transaction>>;

    fn transactions_mut(&mut self) -> Option<&mut Vec<Transaction>>;
}

pub trait BlockHeader: Serialize + Deserialize {
    fn hash(&self) -> Blake2bHash;

    fn height(&self) -> u32;

    /// Time since unix epoch in milliseconds
    fn timestamp(&self) -> u64;
}

pub trait BlockError: Debug + Clone + PartialEq + Eq + Fail + Send + Sync + 'static {}
