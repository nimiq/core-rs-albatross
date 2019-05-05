use std::fmt;
use std::io;

use account::AccountReceipt;
use beserial::{Deserialize, Serialize};
use crate::BlockError;
use crate::slash::SlashInherent;
use crate::view_change::{ViewChange, ViewChangeProof};
use hash::{Hash, Blake2bHash, SerializeContent};
use nimiq_bls::bls12_381::Signature;
use primitives::networks::NetworkId;
use std::cmp::Ordering;
use transaction::Transaction;

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
    pub view_number: u16,

    pub parent_hash: Blake2bHash,
    pub extrinsics_root: Blake2bHash,
    pub state_root: Blake2bHash,

    pub seed: Signature,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroJustification {
    pub signature: Signature,
    pub view_change_proof: Option<ViewChangeProof>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroExtrinsics {
    #[beserial(len_type(u16))]
    pub slash_inherents: Vec<SlashInherent>,

    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    #[beserial(len_type(u16))]
    pub account_receipts: Vec<AccountReceipt>,
}

impl MicroBlock {
    pub fn verify(&self, network_id: NetworkId) -> bool {
        if let Some(ref extrinsics) = self.extrinsics {
            if !extrinsics.verify(self.header.block_number, network_id).is_err() {
                return false;
            }
        }

        if self.header.view_number >= 1 && self.justification.view_change_proof.is_none() {
            return false;
        }

        if let Some(view_change_proof) = &self.justification.view_change_proof {
            // check view change proof
        }

        return true;
    }
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

        let mut previous_acc: Option<&AccountReceipt> = None;
        for acc in &self.account_receipts {
            // Ensure pruned accounts are ordered and unique.
            if let Some(previous) = previous_acc {
                match previous.cmp(acc) {
                    Ordering::Equal => {
                        return Err(BlockError::DuplicateAccountReceipt);
                    }
                    Ordering::Greater => {
                        return Err(BlockError::AccountReceiptsNotOrdered);
                    }
                    _ => (),
                }
            }
            previous_acc = Some(acc);

            // Check that the account is actually supposed to be pruned.
            match acc {
                AccountReceipt::Pruned(acc) => {
                    if !acc.account.is_to_be_pruned() {
                        return Err(BlockError::InvalidAccountReceipt);
                    }
                },
            }
        }

        return Ok(());
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
