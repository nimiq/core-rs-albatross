use std::{cmp::Ordering, collections::HashSet, fmt, fmt::Debug};

use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::{Ed25519PublicKey, Ed25519Signature};
use nimiq_primitives::{networks::NetworkId, policy::Policy, slots_allocation::Validators};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize, SerializedSize};
use nimiq_transaction::{ExecutedTransaction, Transaction};
use nimiq_vrf::VrfSeed;

use crate::{
    equivocation_proof::EquivocationProof, skip_block::SkipBlockProof, BlockError, SkipBlockInfo,
};

/// The struct representing a Micro block.
#[derive(
    Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize, DbSerializable,
)]
pub struct MicroBlock {
    /// The header, contains some basic information and commitments to the body and the state.
    pub header: MicroHeader,
    /// The justification, contains all the information needed to verify that the header was signed
    /// by the correct producer.
    pub justification: Option<MicroJustification>,
    /// The body of the micro-block.
    pub body: Option<MicroBody>,
}

impl MicroBlock {
    /// Returns the network ID of this micro block.
    pub fn network(&self) -> NetworkId {
        self.header.network
    }

    /// Returns the hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }

    /// Returns the block number of this micro block.
    pub fn block_number(&self) -> u32 {
        self.header.block_number
    }

    /// Returns the batch number of this micro block.
    pub fn batch_number(&self) -> u32 {
        Policy::batch_at(self.header.block_number)
    }

    /// Returns the epoch number of this micro block.
    pub fn epoch_number(&self) -> u32 {
        Policy::epoch_at(self.header.block_number)
    }

    /// Returns whether the micro block is a skip block
    pub fn is_skip_block(&self) -> bool {
        if let Some(justification) = &self.justification {
            return matches!(justification, MicroJustification::Skip(_));
        }
        false
    }

    /// Returns the available size, in bytes, in a micro block body for transactions.
    pub fn get_available_bytes(num_equivocation_proofs: usize) -> usize {
        #[allow(clippy::identity_op)]
        Policy::MAX_SIZE_MICRO_BODY.saturating_sub(
            0
            + /* equivocation_proofs vector length */ 2
            + num_equivocation_proofs * EquivocationProof::MAX_SIZE
            + /* transactions vector length */ 2,
        )
    }

    pub(crate) fn verify_proposer(&self, signing_key: &Ed25519PublicKey) -> Result<(), BlockError> {
        let justification = self
            .justification
            .as_ref()
            .ok_or(BlockError::MissingJustification)?;

        match justification {
            MicroJustification::Micro(signature) => {
                let hash = self.hash();
                if !signing_key.verify(signature, hash.as_slice()) {
                    warn!(
                        block = %self,
                        %signing_key,
                        reason = "Invalid signature for proposer",
                        "Rejecting block"
                    );
                    return Err(BlockError::InvalidJustification);
                }
            }
            MicroJustification::Skip(_) => {}
        };

        Ok(())
    }

    pub(crate) fn verify_validators(&self, validators: &Validators) -> Result<(), BlockError> {
        let justification = self
            .justification
            .as_ref()
            .ok_or(BlockError::MissingJustification)?;

        match justification {
            MicroJustification::Skip(proof) => {
                let skip_block = SkipBlockInfo {
                    block_number: self.header.block_number,
                    vrf_entropy: self.header.seed.entropy(),
                };

                if !proof.verify(&skip_block, validators) {
                    debug!(
                        block = %self,
                        reason = "Bad skip block proof",
                        "Rejecting block"
                    );
                    return Err(BlockError::InvalidSkipBlockProof);
                }
            }
            MicroJustification::Micro(_) => {}
        }

        Ok(())
    }
}

impl fmt::Display for MicroBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.header, f)
    }
}

/// Enumeration representing the justification for a Micro block
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
#[serde(rename_all = "camelCase")]
#[repr(u8)]
pub enum MicroJustification {
    /// Regular micro block justification which is the signature of the block producer
    Micro(Ed25519Signature),
    /// Skip block justification which is the aggregated signature of the validator's
    /// signatures for a skip block.
    Skip(SkipBlockProof),
}

impl MicroJustification {
    pub fn unwrap_micro(self) -> Ed25519Signature {
        match self {
            MicroJustification::Micro(signature) => signature,
            MicroJustification::Skip(_) => unreachable!(),
        }
    }
}
/// The struct representing the header of a Micro block.
#[derive(Clone, Debug, Default, Serialize, Deserialize, SerializeContent)]
pub struct MicroHeader {
    /// Network of the block.
    pub network: NetworkId,
    /// The version number of the block. Changing this always results in a hard fork.
    pub version: u16,
    /// The number of the block.
    pub block_number: u32,
    /// The timestamp of the block. It follows the Unix time and has millisecond precision.
    pub timestamp: u64,
    /// The hash of the header of the immediately preceding block (either micro or macro).
    pub parent_hash: Blake2bHash,
    /// The seed of the block. This is the BLS signature of the seed of the immediately preceding
    /// block (either micro or macro) using the validator key of the block producer.
    pub seed: VrfSeed,
    /// The extra data of the block. It is simply 32 raw bytes. No planned use.
    pub extra_data: Vec<u8>,
    /// The root of the Merkle tree of the blockchain state. It just acts as a commitment to the
    /// state.
    pub state_root: Blake2bHash,
    /// The root of the Merkle tree of the body. It just acts as a commitment to the
    /// body.
    pub body_root: Blake2sHash,
    /// The root of the trie diff tree proof.
    pub diff_root: Blake2bHash,
    /// A Merkle root over all the transactions that happened in the current epoch.
    pub history_root: Blake2bHash,
    /// The cached hash of this header. This is NOT sent over the wire.
    #[serde(skip)]
    pub cached_hash: Option<Blake2bHash>,
}

impl MicroHeader {
    /// Returns the Blake2b hash of this header.
    pub fn hash(&self) -> Blake2bHash {
        if let Some(hash) = &self.cached_hash {
            return hash.clone();
        }
        Hash::hash(&self)
    }

    /// Returns the Blake2b hash of this header and caches the result internally.
    pub fn hash_cached(&mut self) -> Blake2bHash {
        if self.cached_hash.is_none() {
            self.cached_hash = Some(Hash::hash(self));
        }
        self.cached_hash.as_ref().unwrap().clone()
    }
}

impl SerializedMaxSize for MicroHeader {
    /// Returns the size, in bytes, of a Micro block header. This represents the maximum possible
    /// size since we assume that the extra_data field is completely filled.
    #[allow(clippy::identity_op)]
    const MAX_SIZE: usize = 0
        + /*network*/ NetworkId::SIZE
        + /*version*/ u16::MAX_SIZE
        + /*block_number*/ u32::MAX_SIZE
        + /*timestamp*/ u64::MAX_SIZE
        + /*parent_hash*/ Blake2bHash::SIZE
        + /*seed*/ VrfSeed::SIZE
        + /*extra_data*/ nimiq_serde::seq_max_size(u8::SIZE, 32)
        + /*state_root*/ Blake2bHash::SIZE
        + /*body_root*/ Blake2sHash::SIZE
        + /*diff_root*/ Blake2bHash::SIZE
        + /*history_root*/ Blake2bHash::SIZE;
}

// We can't derive this because we want to ignore the `cached_hash` field.
impl PartialEq for MicroHeader {
    fn eq(&self, other: &Self) -> bool {
        self.network == other.network
            && self.version == other.version
            && self.block_number == other.block_number
            && self.timestamp == other.timestamp
            && self.parent_hash == other.parent_hash
            && self.seed == other.seed
            && self.extra_data == other.extra_data
            && self.state_root == other.state_root
            && self.body_root == other.body_root
            && self.diff_root == other.diff_root
            && self.history_root == other.history_root
    }
}

impl Eq for MicroHeader {}

impl fmt::Display for MicroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "#{}:MI:{}",
            self.block_number,
            self.hash().to_short_str(),
        )
    }
}

/// The struct representing the body of a Micro block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, SerializeContent)]
pub struct MicroBody {
    /// A vector containing the equivocation proofs for this block. It might be empty.
    pub equivocation_proofs: Vec<EquivocationProof>,
    /// A vector containing the transactions for this block. It might be empty.
    pub transactions: Vec<ExecutedTransaction>,
}

impl SerializedMaxSize for MicroBody {
    const MAX_SIZE: usize = Policy::MAX_SIZE_MICRO_BODY;
}

impl MicroBody {
    /// Obtains the raw transactions contained in this micro block
    pub fn get_raw_transactions(&self) -> Vec<Transaction> {
        // Extract the transactions from the block
        self.transactions
            .iter()
            .map(|txn| txn.get_raw_transaction().clone())
            .collect()
    }

    /// Verifies the micro block: size, proofs, transactions, etc.
    pub(crate) fn verify(&self, is_skip: bool, block_number: u32) -> Result<(), BlockError> {
        // Check that the maximum body size is not exceeded.
        let body_size = self.serialized_size();
        if body_size > Policy::MAX_SIZE_MICRO_BODY {
            debug!(
                body_size = body_size,
                max_size = Policy::MAX_SIZE_MICRO_BODY,
                reason = "Body size exceeds maximum size",
                "Invalid block"
            );
            return Err(BlockError::SizeExceeded);
        }

        // Check that the body is empty for skip blocks.
        if is_skip && (!self.equivocation_proofs.is_empty() || !self.transactions.is_empty()) {
            debug!(
                num_transactions = self.transactions.len(),
                num_equivocation_proofs = self.equivocation_proofs.len(),
                reason = "Skip block has a non empty body",
                "Invalid block"
            );
            return Err(BlockError::InvalidSkipBlockBody);
        }

        // Ensure that equivocation proofs are ordered, unique and within their reporting window.
        let mut previous_proof: Option<&EquivocationProof> = None;
        for proof in &self.equivocation_proofs {
            // Check reporting window.
            if !proof.is_valid_at(block_number) {
                return Err(BlockError::InvalidForkProof);
            }

            // Check proof ordering and uniqueness.
            if let Some(previous) = previous_proof {
                match previous.sort_key().cmp(&proof.sort_key()) {
                    Ordering::Equal => {
                        return Err(BlockError::DuplicateForkProof);
                    }
                    Ordering::Greater => {
                        return Err(BlockError::ForkProofsNotOrdered);
                    }
                    _ => {}
                }
            }
            previous_proof = Some(proof);
        }

        // Ensure transactions are unique and within their validity window.
        let mut uniq = HashSet::new();
        for tx in &self.get_raw_transactions() {
            // Check validity window.
            if !tx.is_valid_at(block_number) {
                return Err(BlockError::ExpiredTransaction);
            }

            // Check uniqueness.
            if !uniq.insert(tx.hash::<Blake2bHash>()) {
                return Err(BlockError::DuplicateTransaction);
            }
        }

        Ok(())
    }
}
