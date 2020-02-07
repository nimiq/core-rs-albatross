use std::cmp::Ordering;
use std::fmt;

use beserial::{Deserialize, Serialize};
use bls::CompressedSignature;
use hash::{Blake2bHash, Hash, SerializeContent};
use hash_derive::SerializeContent;
use primitives::networks::NetworkId;
use transaction::Transaction;
use vrf::VrfSeed;

use crate::fork_proof::ForkProof;
use crate::{BlockError, ViewChangeProof};

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

    pub seed: VrfSeed,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroJustification {
    pub signature: CompressedSignature,
    pub view_change_proof: Option<ViewChangeProof>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, SerializeContent)]
pub struct MicroExtrinsics {
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub fork_proofs: Vec<ForkProof>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
}

impl MicroBlock {
    pub const MAX_SIZE: usize = 100_000; // 100 KB

    pub fn verify(&self, network_id: NetworkId) -> Result<(), BlockError> {
        if let Some(ref extrinsics) = self.extrinsics {
            extrinsics.verify(self.header.block_number, network_id)?;

            if self.header.extrinsics_root != extrinsics.hash() {
                return Err(BlockError::BodyHashMismatch);
            }
        }

        Ok(())
    }

    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
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

        Ok(())
    }

    pub fn get_metadata_size(num_fork_proofs: usize, extra_data_size: usize) -> usize {
        /*fork_proofs size*/
        2
            + num_fork_proofs * ForkProof::SIZE
            + /*extra_data size*/ 1
            + extra_data_size
            + /*transactions size*/ 2
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MicroHeader {}

impl fmt::Display for MicroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "[#{} view {}, type Micro]",
            self.block_number, self.view_number
        )
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MicroExtrinsics {}
