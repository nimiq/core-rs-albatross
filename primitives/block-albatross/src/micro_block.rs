use std::fmt;

use account::Receipt;
use beserial::{Deserialize, Serialize};
use crate::BlockError;
use crate::fork_proof::ForkProof;
use crate::view_change::ViewChange;
use hash::{Hash, Blake2bHash, SerializeContent};
use primitives::networks::NetworkId;
use nimiq_bls::bls12_381::CompressedSignature;
use std::cmp::Ordering;
use transaction::Transaction;
use crate::signed;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroBlock {
    pub header: MicroHeader,
    pub justification: MicroJustification,
    pub extrinsics: Option<MicroExtrinsics>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, SerializeContent)]
pub struct MicroHeader {
    pub version: u16,

    // Digest
    pub block_number: u32,
    pub view_number: u32,

    pub parent_hash: Blake2bHash,
    pub extrinsics_root: Blake2bHash,
    pub state_root: Blake2bHash,

    pub seed: CompressedSignature,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroJustification {
    pub signature: CompressedSignature,
    pub view_change_proof: Option<signed::AggregateProof<ViewChange>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, SerializeContent)]
pub struct MicroExtrinsics {
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub fork_proofs: Vec<ForkProof>,
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

            if self.header.extrinsics_root != extrinsics.hash() {
                return Err(BlockError::BodyHashMismatch);
            }
        }

        Ok(())
    }

    pub fn view_change(&self) -> Option<ViewChange> {
        if self.justification.view_change_proof.is_some() {
            Some(ViewChange {
                block_number: self.header.block_number,
                new_view_number: self.header.view_number,
            })
        } else {
            None
        }
    }
}

impl MicroHeader {
    pub const SIZE: usize = /*version*/ 2 + /*block_number*/ 4 + /*view_number*/ 4
        + /*hashes*/ 3 * 32 + /*seed*/ 48 + /*timestamp*/ 8;
}

impl MicroExtrinsics {
    pub fn verify(&self, block_height: u32, network_id: NetworkId) -> Result<(), BlockError> {
        // Verify fork proofs.
        let mut previous_proof: Option<&ForkProof> = None;
        for proof in &self.fork_proofs {
            // Ensure proofs are ordered and unique.
            if let Some(previous) = previous_proof {
                match previous.cmp(proof) {
                    Ordering::Equal => {
                        return Err(BlockError::DuplicateForkProof);
                    }
                    Ordering::Greater => {
                        return Err(BlockError::ForkProofsNotOrdered);
                    }
                    _ => (),
                }
            }
            previous_proof = Some(proof);

            // Check that the proof is within the reporting window.
            if !proof.is_valid_at(block_height) {
                return Err(BlockError::InvalidForkProof);
            }
        }

        // Verify transactions.
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

        // Verify receipts.
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

            match receipt {
                Receipt::PrunedAccount(acc) => {
                    // Check that the account is actually supposed to be pruned.
                    if !acc.account.is_to_be_pruned() {
                        return Err(BlockError::InvalidReceipt);
                    }
                },
                Receipt::Transaction { index, .. } => {
                    // Check receipt index.
                    if *index >= self.transactions.len() as u16 {
                        return Err(BlockError::InvalidReceipt);
                    }
                },
                Receipt::Inherent { .. } => {
                    // We can't check the index here as we don't know the inherents.
                }
            }
        }

        return Ok(());
    }

    pub fn get_metadata_size(num_fork_proofs: usize, extra_data_size: usize) -> usize {
        return /*fork_proofs size*/ 2
            + num_fork_proofs * ForkProof::SIZE
            + /*extra_data size*/ 1
            + extra_data_size
            + /*transactions size*/ 2
            + /*receipts size*/ 2;
    }
}

impl Hash for MicroHeader { }

impl fmt::Display for MicroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{} view {}, type Micro]", self.block_number, self.view_number)
    }
}

impl Hash for MicroExtrinsics { }
