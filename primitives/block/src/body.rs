use std::{cmp::Ordering, io};

use account::Receipt;
use account::inherent::{Inherent, InherentType};
use beserial::{Deserialize, Serialize};
use hash::{Hash, HashOutput, SerializeContent};
use keys::Address;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use primitives::policy;
use transaction::Transaction;
use utils::merkle;

use crate::BlockError;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BlockBody {
    pub miner: Address,
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    #[beserial(len_type(u16))]
    pub receipts: Vec<Receipt>,
}

impl SerializeContent for BlockBody {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

// Different hash implementation than std
#[allow(clippy::derive_hash_xor_eq)]
impl Hash for BlockBody {
    fn hash<H: HashOutput>(&self) -> H {
        let vec = self.get_merkle_leaves();
        merkle::compute_root_from_hashes::<H>(&vec)
    }
}

#[allow(unreachable_code)]
impl BlockBody {
    pub fn verify(&self, block_height: u32, network_id: NetworkId) -> Result<(), BlockError> {
        let mut total_fees = Coin::ZERO;

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

            // Check for fee overflow.
            total_fees = total_fees.checked_add(tx.fee)
                .ok_or(BlockError::FeeOverflow)?;
        }

        let mut previous_receipt: Option<&Receipt> = None;
        for receipt in &self.receipts {
            // Ensure receipts are ordered and unique.
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
                _ => return Err(BlockError::InvalidReceipt)
            }
        }

        // Everything checks out.
        Ok(())
    }

    pub fn get_merkle_leaves<H: HashOutput>(&self) -> Vec<H> {
        let mut vec: Vec<H> = Vec::with_capacity(2 + self.transactions.len() + self.receipts.len());
        vec.push(self.miner.hash());
        vec.push(self.extra_data.hash());
        for t in &self.transactions {
            vec.push(t.hash());
        }
        for p in &self.receipts {
            vec.push(p.hash());
        }
        vec
    }

    /// Can panic on unverified blocks if the total fees overflow.
    pub fn total_fees(&self) -> Coin {
        self.transactions.iter().fold(Coin::ZERO, |sum, tx| sum + tx.fee)
    }

    /// Can panic on unverified blocks if the total fees overflow.
    // XXX Does this really belong here?
    pub fn get_reward_inherent(&self, block_height: u32) -> Inherent {
        Inherent {
            ty: InherentType::Reward,
            target: self.miner.clone(),
            value: policy::block_reward_at(block_height) + self.total_fees(),
            data: vec![]
        }
    }

    pub fn get_metadata_size(extra_data_size: usize) -> usize {
        return Address::SIZE
            + /*extra_data size*/ 1
            + extra_data_size
            + /*transactions size*/ 2
            + /*receipts size*/ 2;
    }
}
