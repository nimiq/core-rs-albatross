use std::fmt;
use std::io;

use account::Receipt;
use beserial::{Deserialize, Serialize};
use crate::BlockError;
use crate::fork_proof::ForkProof;
use crate::view_change::{ViewChange, ViewChangeProof};
use hash::{Hash, Blake2bHash, SerializeContent};
use primitives::networks::NetworkId;
use nimiq_bls::bls12_381::Signature;
use std::cmp::Ordering;
use transaction::Transaction;
use crate::signed;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroBlock {
    pub header: MicroHeader,
    pub justification: MicroJustification,
    pub extrinsics: Option<MicroExtrinsics>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroHeader {
    pub version: u16,

    // Digest
    pub block_number: u32,
    pub view_number: u32,

    pub parent_hash: Blake2bHash,
    pub extrinsics_root: Blake2bHash,
    pub state_root: Blake2bHash,

    pub seed: Signature,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroJustification {
    pub signature: Signature,
    pub view_change_proof: Option<signed::UntrustedAggregateProof<ViewChange>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroExtrinsics {
    #[beserial(len_type(u16))]
    pub fork_proofs: Vec<ForkProof>,

    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    #[beserial(len_type(u16))]
    pub receipts: Vec<Receipt>,
}

impl MicroBlock {
    pub const MAX_SIZE: usize = 100_000; // 100 kb

    pub fn verify(&self, network_id: NetworkId) -> Result<(), BlockError> {
        if let Some(ref extrinsics) = self.extrinsics {
            extrinsics.verify(self.header.block_number, network_id)?;
        }

        if self.header.view_number >= 1 && self.justification.view_change_proof.is_none() {
            return Err(BlockError::NoViewChangeProof);
        }

        if let Some(view_change_proof) = &self.justification.view_change_proof {
            // check view change proof
        }

        Ok(())
    }

    pub fn serialize_without_signature(&self) -> Vec<u8> {
        unimplemented!()
    }
}

impl MicroHeader {
    pub const SIZE: usize = /*version*/ 2 + /*block_number*/ 4 + /*view_number*/ 4
        + /*hashes*/ 3 * 32 + /*seed*/ 48 + /*timestamp*/ 8;
}

impl MicroExtrinsics {
    pub fn verify(&self, block_height: u32, network_id: NetworkId) -> Result<(), BlockError> {
        let mut previous_tx: Option<&Transaction> = None;
        for tx in &self.transactions {
            // Ensure transactions are ordered and unique.
            if let Some(previous) = previous_tx {
                match previous.cmp_block_order(tx) {
                    Ordering::Equal => {
                        return Err(BlockError::DuplicateTransaction);
                    }
                    Ordering::Greater => {
                        return Err(BlockError::TransactionsNotOrdered);
                    }
                    _ => (),
                }
            }
            previous_tx = Some(tx);

            // Check that the transaction is within its validity window.
            if !tx.is_valid_at(block_height) {
                return Err(BlockError::ExpiredTransaction);
            }

            // Check intrinsic transaction invariants.
            if let Err(e) = tx.verify(network_id) {
                return Err(BlockError::InvalidTransaction(e));
            }
        }

        let mut previous_receipt: Option<&Receipt> = None;
        for receipt in &self.receipts {
            // Ensure pruned accounts are ordered and unique.
            if let Some(previous) = previous_receipt {
                match previous.cmp(receipt) {
                    Ordering::Equal => {
                        return Err(BlockError::DuplicateReceipt);
                    }
                    Ordering::Greater => {
                        return Err(BlockError::ReceiptsNotOrdered);
                    }
                    _ => (),
                }
            }
            previous_receipt = Some(receipt);

            // Check that the account is actually supposed to be pruned.
            match receipt {
                Receipt::PrunedAccount(acc) => {
                    if !acc.account.is_to_be_pruned() {
                        return Err(BlockError::InvalidReceipt);
                    }
                },
                Receipt::Transaction {..} => {
                    // TODO
                    unimplemented!()
                },
                Receipt::Inherent {..} => {
                    // TODO
                    unimplemented!()
                }
            }
        }

        return Ok(());
    }

    pub fn get_metadata_size(num_fork_proofs: usize, extra_data_size: usize) -> usize {
        return /*fork_proofs size*/ 2 + num_fork_proofs * ForkProof::SIZE
            + /*extra_data size*/ 1
            + extra_data_size
            + /*transactions size*/ 2
            + /*receipts size*/ 2;
    }
}

impl SerializeContent for MicroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MicroHeader { }

impl fmt::Display for MicroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{} view {}, type Micro]", self.block_number, self.view_number)
    }
}

impl SerializeContent for MicroExtrinsics {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MicroExtrinsics { }
