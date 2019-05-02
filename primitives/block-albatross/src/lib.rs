#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_account as account;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
#[macro_use]
extern crate nimiq_macros as macros;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

mod block;
mod macro_block;
mod micro_block;
mod pbft;
mod slash;

use crate::transaction::TransactionError;
use beserial::Deserialize;
use pairing::bls12_381::Bls12;
use beserial::SerializingError;
use beserial::WriteBytesExt;
use beserial::Serialize;
use beserial::ReadBytesExt;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockError {
    UnsupportedVersion,
    FromTheFuture,
    InvalidPoW,
    SizeExceeded,
    InterlinkHashMismatch,
    BodyHashMismatch,

    DuplicateTransaction,
    InvalidTransaction(TransactionError),
    ExpiredTransaction,
    TransactionsNotOrdered,

    DuplicatePrunedAccount,
    PrunedAccountsNotOrdered,
    InvalidPrunedAccount,
}

#[derive(Clone, Debug)]
pub struct AggregateProof {
    pub signatures: nimiq_bls::bls12_381::AggregateSignature,
    pub keys: nimiq_bls::bls12_381::AggregatePublicKey
}

impl PartialEq for AggregateProof {
    fn eq(&self, other: &AggregateProof) -> bool {
        unimplemented!()
    }
}

impl Serialize for AggregateProof {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}

impl Deserialize for AggregateProof {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        unimplemented!()
    }
}

impl Eq for AggregateProof { }
