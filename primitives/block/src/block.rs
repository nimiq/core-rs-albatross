use std::fmt;

use nimiq_bls::cache::PublicKeyCache;
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::Ed25519PublicKey;
use nimiq_network_interface::network::Topic;
use nimiq_primitives::{
    coin::Coin, networks::NetworkId, policy::Policy, slots_allocation::Validators,
};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};
use nimiq_transaction::ExecutedTransaction;
use nimiq_vrf::VrfSeed;

use crate::{
    macro_block::MacroBlock, micro_block::MicroBlock, BlockError, MacroBody, MicroBody,
    MicroJustification, TendermintProof,
};

/// These network topics are used to subscribe and request Blocks and Block Headers respectively
#[derive(Clone, Debug, Default)]
pub struct BlockTopic;

impl Topic for BlockTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block";
    const VALIDATE: bool = true;
}

#[derive(Clone, Debug, Default)]
pub struct BlockHeaderTopic;

impl Topic for BlockHeaderTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block-header";
    const VALIDATE: bool = true;
}

/// Defines the type of the block, either Micro or Macro (which includes both checkpoint and
/// election blocks).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockType {
    Macro = 1,
    Micro = 2,
}

impl BlockType {
    pub fn of(block_number: u32) -> Self {
        if Policy::is_macro_block_at(block_number) {
            BlockType::Macro
        } else {
            BlockType::Micro
        }
    }
}

/// The enum representing a block. Blocks can either be Micro blocks or Macro blocks (which includes
/// both checkpoint and election blocks).
#[derive(
    Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize, DbSerializable,
)]
pub enum Block {
    Macro(MacroBlock),
    Micro(MicroBlock),
}

impl Block {
    /// Returns the type of the block.
    pub fn ty(&self) -> BlockType {
        match self {
            Block::Macro(_) => BlockType::Macro,
            Block::Micro(_) => BlockType::Micro,
        }
    }

    /// Returns the network ID of the block.
    pub fn network(&self) -> NetworkId {
        match self {
            Block::Macro(ref block) => block.header.network,
            Block::Micro(ref block) => block.header.network,
        }
    }

    /// Returns the version number of the block.
    pub fn version(&self) -> u16 {
        match self {
            Block::Macro(ref block) => block.header.version,
            Block::Micro(ref block) => block.header.version,
        }
    }

    /// Returns the block number of the block.
    pub fn block_number(&self) -> u32 {
        match self {
            Block::Macro(ref block) => block.header.block_number,
            Block::Micro(ref block) => block.header.block_number,
        }
    }

    /// Returns the batch number of the block.
    pub fn batch_number(&self) -> u32 {
        Policy::batch_at(self.block_number())
    }

    /// Returns the epoch number of the block.
    pub fn epoch_number(&self) -> u32 {
        Policy::epoch_at(self.block_number())
    }

    /// Returns the timestamp of the block.
    pub fn timestamp(&self) -> u64 {
        match self {
            Block::Macro(ref block) => block.header.timestamp,
            Block::Micro(ref block) => block.header.timestamp,
        }
    }

    /// Returns the parent hash of the block. The parent hash is the hash of the header of the
    /// immediately preceding block.
    pub fn parent_hash(&self) -> &Blake2bHash {
        match self {
            Block::Macro(ref block) => &block.header.parent_hash,
            Block::Micro(ref block) => &block.header.parent_hash,
        }
    }

    /// Returns the parent election hash of the block. The parent election hash is the hash of the
    /// header of the preceding election macro block.
    pub fn parent_election_hash(&self) -> Option<&Blake2bHash> {
        match self {
            Block::Macro(ref block) => Some(&block.header.parent_election_hash),
            Block::Micro(ref _block) => None,
        }
    }

    /// Returns the seed of the block.
    pub fn seed(&self) -> &VrfSeed {
        match self {
            Block::Macro(ref block) => &block.header.seed,
            Block::Micro(ref block) => &block.header.seed,
        }
    }

    /// Returns the extra data of the block.
    pub fn extra_data(&self) -> &[u8] {
        match self {
            Block::Macro(ref block) => &block.header.extra_data,
            Block::Micro(ref block) => &block.header.extra_data,
        }
    }

    /// Returns the state root of the block.
    pub fn state_root(&self) -> &Blake2bHash {
        match self {
            Block::Macro(ref block) => &block.header.state_root,
            Block::Micro(ref block) => &block.header.state_root,
        }
    }

    /// Returns the body root of the block.
    pub fn body_root(&self) -> &Blake2sHash {
        match self {
            Block::Macro(ref block) => &block.header.body_root,
            Block::Micro(ref block) => &block.header.body_root,
        }
    }

    /// Returns the history root of the block.
    pub fn history_root(&self) -> &Blake2bHash {
        match self {
            Block::Macro(ref block) => &block.header.history_root,
            Block::Micro(ref block) => &block.header.history_root,
        }
    }

    /// Returns the diff root of the block.
    pub fn diff_root(&self) -> &Blake2bHash {
        match self {
            Block::Macro(block) => &block.header.diff_root,
            Block::Micro(block) => &block.header.diff_root,
        }
    }

    /// Returns the Blake2b hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        match self {
            Block::Macro(ref block) => block.header.hash(),
            Block::Micro(ref block) => block.header.hash(),
        }
    }

    /// Returns a copy of the validators. Only returns Some if it is an election block.
    pub fn validators(&self) -> Option<Validators> {
        match self {
            Block::Macro(block) => block.get_validators(),
            Block::Micro(_) => None,
        }
    }

    /// Returns the offset used as input to the VRF when determining the proposer of this block.
    /// For macro blocks, this returns the round where the block was originally proposed (as
    /// opposed to the round where the block was accepted).
    pub fn vrf_offset(&self) -> u32 {
        match self {
            Block::Macro(block) => block.round(),
            Block::Micro(block) => block.block_number(),
        }
    }

    /// Returns the justification of the block. If the block has no justification then it returns
    /// None.
    pub fn justification(&self) -> Option<BlockJustification> {
        // TODO Can we eliminate the clone()s here?
        Some(match self {
            Block::Macro(ref block) => {
                BlockJustification::Macro(block.justification.as_ref()?.clone())
            }
            Block::Micro(ref block) => {
                BlockJustification::Micro(block.justification.as_ref()?.clone())
            }
        })
    }

    /// Returns the body of the block. If the block has no body then it returns None.
    pub fn body(&self) -> Option<BlockBody> {
        // TODO Can we eliminate the clone()s here?
        match self {
            Block::Macro(ref block) => Some(BlockBody::Macro(block.body.as_ref()?.clone())),
            Block::Micro(ref block) => Some(BlockBody::Micro(block.body.as_ref()?.clone())),
        }
    }

    /// Returns a reference to the transactions of the block. If the block is a Macro block it just
    /// returns None, since Macro blocks don't contain any transactions.
    pub fn transactions(&self) -> Option<&[ExecutedTransaction]> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref block) => block.body.as_ref().map(|ex| &ex.transactions[..]),
        }
    }

    /// Returns a mutable reference to the transactions of the block. If the block is a Macro block
    /// it just returns None, since Macro blocks don't contain any transactions.
    pub fn transactions_mut(&mut self) -> Option<&mut Vec<ExecutedTransaction>> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref mut block) => block.body.as_mut().map(|ex| &mut ex.transactions),
        }
    }

    /// Return the number of transactions in the block. If the block is a Macro
    /// block it just returns zero, since Macro blocks don't contain any transactions.
    pub fn num_transactions(&self) -> usize {
        self.transactions().map_or(0, |txs| txs.len())
    }

    /// Returns the sum of the fees of all of the transactions in the block. If the block is a Macro
    /// block it just returns zero, since Macro blocks don't contain any transactions.
    pub fn sum_transaction_fees(&self) -> Coin {
        match self {
            Block::Macro(_) => Coin::ZERO,
            Block::Micro(ref block) => block
                .body
                .as_ref()
                .map(|ex| {
                    ex.transactions
                        .iter()
                        .map(|tx| tx.get_raw_transaction().fee)
                        .sum()
                })
                .unwrap_or(Coin::ZERO),
        }
    }

    /// Unwraps the block and returns a reference to the underlying Macro block.
    pub fn unwrap_macro_ref(&self) -> &MacroBlock {
        if let Block::Macro(ref block) = self {
            block
        } else {
            unreachable!()
        }
    }

    /// Unwraps the block and returns a reference to the underlying Micro block.
    pub fn unwrap_micro_ref(&self) -> &MicroBlock {
        if let Block::Micro(ref block) = self {
            block
        } else {
            unreachable!()
        }
    }

    /// Unwraps the block and returns a mutable reference to the underlying Macro block.
    pub fn unwrap_macro_ref_mut(&mut self) -> &mut MacroBlock {
        if let Block::Macro(ref mut block) = self {
            block
        } else {
            unreachable!()
        }
    }

    /// Unwraps the block and returns a mutable reference to the underlying Micro block.
    pub fn unwrap_micro_ref_mut(&mut self) -> &mut MicroBlock {
        if let Block::Micro(ref mut block) = self {
            block
        } else {
            unreachable!()
        }
    }

    /// Unwraps the block and returns the underlying Macro block.
    pub fn unwrap_macro(self) -> MacroBlock {
        if let Block::Macro(block) = self {
            block
        } else {
            unreachable!()
        }
    }

    /// Unwraps the block and returns the underlying Micro block.
    pub fn unwrap_micro(self) -> MicroBlock {
        if let Block::Micro(block) = self {
            block
        } else {
            unreachable!()
        }
    }

    /// Unwraps the block and returns the underlying transactions. This only works with Micro blocks.
    pub fn unwrap_transactions(self) -> Vec<ExecutedTransaction> {
        self.unwrap_micro().body.unwrap().transactions
    }

    /// Returns true if the block is a Micro block, false otherwise.
    pub fn is_micro(&self) -> bool {
        matches!(self, Block::Micro(_))
    }

    /// Returns true if the block is a Skip block, false otherwise.
    pub fn is_skip(&self) -> bool {
        match self {
            Block::Macro(_) => false,
            Block::Micro(block) => block.is_skip_block(),
        }
    }

    /// Returns true if the block is a Macro block, false otherwise.
    pub fn is_macro(&self) -> bool {
        matches!(self, Block::Macro(_))
    }

    /// Returns true if the block is an election block, false otherwise.
    pub fn is_election(&self) -> bool {
        match self {
            Block::Macro(block) => block.is_election(),
            Block::Micro(_) => false,
        }
    }

    /// Updates validator keys from a public key cache.
    pub fn update_validator_keys(&self, cache: &mut PublicKeyCache) {
        // Prepare validator keys from BLS cache.
        if let Block::Macro(MacroBlock {
            body:
                Some(MacroBody {
                    validators: Some(validators),
                    ..
                }),
            ..
        }) = self
        {
            for validator in validators.iter() {
                cache.get_or_uncompress_lazy_public_key(&validator.voting_key);
            }
        }
    }

    /// Verifies the block.
    /// Note that only intrinsic verifications are performed and further checks
    /// are needed when completely verifying a block.
    pub fn verify(&self, network: NetworkId) -> Result<(), BlockError> {
        // Check the block type.
        if self.ty() != BlockType::of(self.block_number()) {
            return Err(BlockError::InvalidBlockType);
        }

        // Perform header intrinsic verification.
        self.verify_header(network, self.is_skip())?;

        // Verify body if it exists.
        if let Some(body) = self.body() {
            // Check the body root.
            let body_hash = body.hash();
            if *self.body_root() != body_hash {
                warn!(
                    %self,
                    body_root = %self.body_root(),
                    expected_body_hash = %body_hash,
                    reason = "Header body_root doesn't match actual body hash",
                    "Invalid block"
                );
                return Err(BlockError::BodyHashMismatch);
            }

            // Perform block type specific body verification.
            match body {
                BlockBody::Micro(body) => body.verify(self.is_skip(), self.block_number())?,
                BlockBody::Macro(body) => body.verify(self.is_election())?,
            };
        }

        Ok(())
    }

    /// Verifies the header.
    /// Note that only header intrinsic verifications are performed and further checks
    /// are needed when completely verifying a block header.
    pub fn verify_header(&self, network: NetworkId, is_skip: bool) -> Result<(), BlockError> {
        // Check whether the block is from the correct network.
        if self.network() != network {
            return Err(BlockError::NetworkMismatch);
        }

        // Check the version
        if self.version() != Policy::VERSION {
            warn!(
                header = %self,
                obtained_version = self.version(),
                expected_version = Policy::VERSION,
                reason = "wrong version",
                "Invalid block header"
            );

            return Err(BlockError::UnsupportedVersion);
        }

        // Check that the extra data does not exceed the permitted size.
        // This is also checked during deserialization.
        if self.extra_data().len() > 32 {
            warn!(
                header = %self,
                reason = "too much extra data",
                "Invalid block header"
            );
            return Err(BlockError::ExtraDataTooLarge);
        }

        // Check that skip blocks doesn't have any extra data
        if is_skip && !self.extra_data().is_empty() {
            warn!(
                header = %self,
                reason = "Skip block extra data is not empty",
                "Invalid block"
            );
            return Err(BlockError::ExtraDataTooLarge);
        }

        Ok(())
    }

    /// Verifies that the block is a valid immediate successor of the given block.
    pub fn verify_immediate_successor(&self, predecessor: &Block) -> Result<(), BlockError> {
        // Check block number.
        let next_block_number = predecessor.block_number() + 1;
        if self.block_number() != next_block_number {
            debug!(
                block = %self,
                reason = "Wrong block number",
                block_number = self.block_number(),
                expected_block_number = predecessor.block_number(),
                "Rejecting block",
            );
            return Err(BlockError::InvalidBlockNumber);
        }

        // Check parent hash.
        if *self.parent_hash() != predecessor.hash() {
            debug!(
                block = %self,
                reason = "Wrong parent hash",
                parent_hash = %self.parent_hash(),
                expected_parent_hash = %predecessor.hash(),
                "Rejecting block",
            );
            return Err(BlockError::InvalidParentHash);
        }

        // Handle skip blocks.
        if self.is_skip() {
            // Check that skip block has the expected timestamp.
            let expected_timestamp = predecessor.timestamp() + Policy::BLOCK_PRODUCER_TIMEOUT;
            if self.timestamp() != expected_timestamp {
                debug!(
                    block = %self,
                    timestamp = self.timestamp(),
                    expected_timestamp,
                    reason = "Unexpected timestamp for a skip block",
                    "Rejecting block"
                );
                return Err(BlockError::InvalidTimestamp);
            }

            // In skip blocks the VRF seed must be carried over (because a new VRF seed requires a
            // new leader).
            if self.seed() != predecessor.seed() {
                debug!(
                    block = %self,
                    reason = "Invalid seed",
                    "Rejecting skip block"
                );
                return Err(BlockError::InvalidSeed);
            }
        } else {
            // Check that the timestamp is non-decreasing.
            if self.timestamp() < predecessor.timestamp() {
                debug!(
                    block = %self,
                    reason = "Timestamp is decreasing",
                    timestamp = %self.timestamp(),
                    previous_timestamp = %predecessor.timestamp(),
                    "Rejecting block",
                );
                return Err(BlockError::InvalidTimestamp);
            }
        }

        Ok(())
    }

    /// Verifies that the block is valid macro successor to the given macro block.
    /// A macro successor is any macro block between the given macro block (exclusive) and the next
    /// election block (inclusive).
    pub fn verify_macro_successor(&self, predecessor: &MacroBlock) -> Result<(), BlockError> {
        // Check block type.
        if !self.is_macro() {
            return Err(BlockError::InvalidBlockType);
        }

        // Check parent election hash.
        // The expected parent election hash depends on the predecessor:
        // - Predecessor is an election block: this block must have its hash as
        //   parent election hash
        // - Predecessor is a checkpoint block: this block must have the same
        //   parent election hash
        let expected_parent_election_hash = if predecessor.is_election() {
            predecessor.hash()
        } else {
            predecessor.header.parent_election_hash.clone()
        };

        let parent_election_hash = self.parent_election_hash().unwrap();
        if *parent_election_hash != expected_parent_election_hash {
            warn!(
                block = %self,
                %parent_election_hash,
                %expected_parent_election_hash,
                reason = "Wrong parent election hash",
                "Rejecting block"
            );
            return Err(BlockError::InvalidParentElectionHash);
        }

        // Check that block number is in the allowed range.
        let predecessor_block = predecessor.block_number();
        let next_election_block = Policy::election_block_after(predecessor_block);
        if self.block_number() <= predecessor_block || self.block_number() > next_election_block {
            return Err(BlockError::InvalidBlockNumber);
        }

        // Check that the timestamp is non-decreasing.
        // XXX This is also checked in verify_immediate_successor().
        if self.timestamp() < predecessor.header.timestamp {
            debug!(
                block = %self,
                reason = "Timestamp is decreasing",
                timestamp = %self.timestamp(),
                previous_timestamp = %predecessor.header.timestamp,
                "Rejecting block",
            );
            return Err(BlockError::InvalidTimestamp);
        }

        Ok(())
    }

    /// Verifies that the block is valid for the given proposer and VRF seed.
    pub fn verify_proposer(
        &self,
        signing_key: &Ed25519PublicKey,
        prev_seed: &VrfSeed,
    ) -> Result<(), BlockError> {
        // Verify VRF seed.
        if !self.is_skip() {
            self.seed()
                .verify(prev_seed, signing_key)
                .map_err(|_| BlockError::InvalidSeed)?;
        } else {
            // XXX This is also checked in `verify_immediate_successor`.
            if self.seed() != prev_seed {
                return Err(BlockError::InvalidSeed);
            }
        }

        // Verify justification for micro blocks.
        match self {
            Block::Micro(block) => block.verify_proposer(signing_key),
            Block::Macro(_) => Ok(()),
        }
    }

    /// Verifies that the block is valid for the given validators.
    pub fn verify_validators(&self, validators: &Validators) -> Result<(), BlockError> {
        match self {
            Block::Micro(block) => block.verify_validators(validators),
            Block::Macro(block) => block.verify_validators(validators),
        }
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            Block::Macro(block) => fmt::Display::fmt(block, f),
            Block::Micro(block) => fmt::Display::fmt(block, f),
        }
    }
}

/// Struct representing the justification of a block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BlockJustification {
    Micro(MicroJustification),
    Macro(TendermintProof),
}

impl BlockJustification {
    /// Returns the type of the block.
    pub fn ty(&self) -> BlockType {
        match self {
            BlockJustification::Macro(_) => BlockType::Macro,
            BlockJustification::Micro(_) => BlockType::Micro,
        }
    }
}

/// Struct representing the body of a block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BlockBody {
    Micro(MicroBody),
    Macro(MacroBody),
}

impl BlockBody {
    /// Returns the type of the block.
    pub fn ty(&self) -> BlockType {
        match self {
            BlockBody::Macro(_) => BlockType::Macro,
            BlockBody::Micro(_) => BlockType::Micro,
        }
    }

    /// Returns the hash of the body.
    pub fn hash(&self) -> Blake2sHash {
        match self {
            BlockBody::Micro(body) => body.hash(),
            BlockBody::Macro(body) => body.hash(),
        }
    }

    /// Unwraps a block body and returns the underlying Micro body.
    pub fn unwrap_micro(self) -> MicroBody {
        if let BlockBody::Micro(body) = self {
            body
        } else {
            unreachable!()
        }
    }

    /// Unwraps a block body and returns the underlying Macro body.
    pub fn unwrap_macro(self) -> MacroBody {
        if let BlockBody::Macro(body) = self {
            body
        } else {
            unreachable!()
        }
    }
}
