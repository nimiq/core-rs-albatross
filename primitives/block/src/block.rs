use std::convert::TryFrom;
use std::{fmt, io};

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_bls::cache::PublicKeyCache;
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, SerializeContent};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::PublicKey;
use nimiq_network_interface::network::Topic;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy::Policy;
use nimiq_primitives::slots::Validators;
use nimiq_transaction::ExecutedTransaction;
use nimiq_vrf::VrfSeed;

use crate::macro_block::{MacroBlock, MacroHeader};
use crate::micro_block::{MicroBlock, MicroHeader};
use crate::{BlockError, MacroBody, MicroBody, MicroJustification, TendermintProof};

/// These network topics are used to subscribe and request Blocks and Block Headers respectively
#[derive(Clone, Debug, Default)]
pub struct BlockTopic;

impl Topic for BlockTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "blocks";
    const VALIDATE: bool = true;
}

#[derive(Clone, Debug, Default)]
pub struct BlockHeaderTopic;

impl Topic for BlockHeaderTopic {
    type Item = Block;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block-headers";
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub fn extra_data(&self) -> &Vec<u8> {
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
    pub fn body_root(&self) -> &Blake2bHash {
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

    /// Returns the header of the block.
    pub fn header(&self) -> BlockHeader {
        // TODO: Can we eliminate the clone()s here?
        match self {
            Block::Macro(ref block) => BlockHeader::Macro(block.header.clone()),
            Block::Micro(ref block) => BlockHeader::Micro(block.header.clone()),
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
    pub fn transactions(&self) -> Option<&Vec<ExecutedTransaction>> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref block) => block.body.as_ref().map(|ex| &ex.transactions),
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
            Block::Macro(block) => block.is_election_block(),
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
    pub fn verify(&self, check_pk_tree_root: bool) -> Result<(), BlockError> {
        // Check the block type.
        if self.ty() != BlockType::of(self.header().block_number()) {
            return Err(BlockError::InvalidBlockType);
        }

        // Perform header intrinsic verification.
        let header = self.header();
        header.verify(self.is_skip())?;

        // Verify body if it exists.
        if let Some(body) = self.body() {
            // Check the body root.
            let body_hash = body.hash();
            if *header.body_root() != body_hash {
                warn!(
                    %header,
                    body_root = %header.body_root(),
                    expected_body_hash = %body_hash,
                    reason = "Header body_root doesn't match actual body hash",
                    "Invalid block"
                );
                return Err(BlockError::BodyHashMismatch);
            }

            // Perform block type specific body verification.
            match body {
                BlockBody::Micro(body) => body.verify(self.is_skip(), header.block_number())?,
                BlockBody::Macro(body) => body.verify(self.is_election(), check_pk_tree_root)?,
            };
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
        let expected_parent_election_hash = if predecessor.is_election_block() {
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
        signing_key: &PublicKey,
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

impl Serialize for Block {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty().serialize(writer)?;
        size += match self {
            Block::Macro(block) => block.serialize(writer)?,
            Block::Micro(block) => block.serialize(writer)?,
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty().serialized_size();
        size += match self {
            Block::Macro(block) => block.serialized_size(),
            Block::Micro(block) => block.serialized_size(),
        };
        size
    }
}

impl Deserialize for Block {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let block = match ty {
            BlockType::Macro => Block::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => Block::Micro(Deserialize::deserialize(reader)?),
        };
        Ok(block)
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

impl IntoDatabaseValue for Block {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for Block {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

/// The enum representing a block header. Blocks can either be Micro blocks or Macro blocks (which
/// includes both checkpoint and election blocks).
#[derive(Clone, Debug, Eq, PartialEq, SerializeContent)]
pub enum BlockHeader {
    Micro(MicroHeader),
    Macro(MacroHeader),
}

impl BlockHeader {
    /// Returns the type of the block.
    pub fn ty(&self) -> BlockType {
        match self {
            BlockHeader::Macro(_) => BlockType::Macro,
            BlockHeader::Micro(_) => BlockType::Micro,
        }
    }

    /// Returns the version number of the block.
    pub fn version(&self) -> u16 {
        match self {
            BlockHeader::Macro(ref header) => header.version,
            BlockHeader::Micro(ref header) => header.version,
        }
    }

    /// Returns the block number of the block.
    pub fn block_number(&self) -> u32 {
        match self {
            BlockHeader::Macro(ref header) => header.block_number,
            BlockHeader::Micro(ref header) => header.block_number,
        }
    }

    /// Returns the timestamp of the block.
    pub fn timestamp(&self) -> u64 {
        match self {
            BlockHeader::Macro(ref header) => header.timestamp,
            BlockHeader::Micro(ref header) => header.timestamp,
        }
    }

    /// Returns the parent hash of the block. The parent hash is the hash of the header of the
    /// immediately preceding block.
    pub fn parent_hash(&self) -> &Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => &header.parent_hash,
            BlockHeader::Micro(ref header) => &header.parent_hash,
        }
    }

    /// Returns the parent election hash of the block. The parent election hash is the hash of the
    /// header of the preceding election macro block.
    pub fn parent_election_hash(&self) -> Option<&Blake2bHash> {
        match self {
            BlockHeader::Macro(ref header) => Some(&header.parent_election_hash),
            BlockHeader::Micro(ref _header) => None,
        }
    }

    /// Returns the seed of the block.
    pub fn seed(&self) -> &VrfSeed {
        match self {
            BlockHeader::Macro(ref header) => &header.seed,
            BlockHeader::Micro(ref header) => &header.seed,
        }
    }

    /// Returns the extra data of the block.
    pub fn extra_data(&self) -> &Vec<u8> {
        match self {
            BlockHeader::Macro(ref header) => &header.extra_data,
            BlockHeader::Micro(ref header) => &header.extra_data,
        }
    }

    /// Returns the state root of the block.
    pub fn state_root(&self) -> &Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => &header.state_root,
            BlockHeader::Micro(ref header) => &header.state_root,
        }
    }

    /// Returns the body root of the block.
    pub fn body_root(&self) -> &Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => &header.body_root,
            BlockHeader::Micro(ref header) => &header.body_root,
        }
    }

    /// Returns the Blake2b hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => header.hash(),
            BlockHeader::Micro(ref header) => header.hash(),
        }
    }

    /// Returns the Blake2s hash of the block header.
    pub fn hash_blake2s(&self) -> Blake2sHash {
        match self {
            BlockHeader::Macro(ref header) => header.hash(),
            BlockHeader::Micro(ref header) => header.hash(),
        }
    }

    /// Unwraps the header and returns a reference to the underlying Macro header.
    pub fn unwrap_macro_ref(&self) -> &MacroHeader {
        if let BlockHeader::Macro(ref header) = self {
            header
        } else {
            unreachable!()
        }
    }

    /// Unwraps the header and returns a reference to the underlying Micro header.
    pub fn unwrap_micro_ref(&self) -> &MicroHeader {
        if let BlockHeader::Micro(ref header) = self {
            header
        } else {
            unreachable!()
        }
    }

    /// Unwraps the header and returns the underlying Macro header.
    pub fn unwrap_macro(self) -> MacroHeader {
        if let BlockHeader::Macro(header) = self {
            header
        } else {
            unreachable!()
        }
    }

    /// Unwraps the header and returns the underlying Micro header.
    pub fn unwrap_micro(self) -> MicroHeader {
        if let BlockHeader::Micro(header) = self {
            header
        } else {
            unreachable!()
        }
    }

    /// Verifies the header.
    /// Note that only header intrinsic verifications are performed and further checks
    /// are needed when completely verifying a block header.
    pub fn verify(&self, is_skip: bool) -> Result<(), BlockError> {
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
}

impl Hash for BlockHeader {}

impl Serialize for BlockHeader {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty().serialize(writer)?;
        size += match self {
            BlockHeader::Macro(header) => header.serialize(writer)?,
            BlockHeader::Micro(header) => header.serialize(writer)?,
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty().serialized_size();
        size += match self {
            BlockHeader::Macro(header) => header.serialized_size(),
            BlockHeader::Micro(header) => header.serialized_size(),
        };
        size
    }
}

impl Deserialize for BlockHeader {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let header = match ty {
            BlockType::Macro => BlockHeader::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => BlockHeader::Micro(Deserialize::deserialize(reader)?),
        };
        Ok(header)
    }
}

impl fmt::Display for BlockHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            BlockHeader::Macro(header) => fmt::Display::fmt(header, f),
            BlockHeader::Micro(header) => fmt::Display::fmt(header, f),
        }
    }
}

/// Struct representing the justification of a block.
#[derive(Clone, Debug, Eq, PartialEq)]
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

impl Serialize for BlockJustification {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty().serialize(writer)?;
        size += match self {
            BlockJustification::Macro(justification) => justification.serialize(writer)?,
            BlockJustification::Micro(justification) => justification.serialize(writer)?,
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty().serialized_size();
        size += match self {
            BlockJustification::Macro(justification) => justification.serialized_size(),
            BlockJustification::Micro(justification) => justification.serialized_size(),
        };
        size
    }
}

impl Deserialize for BlockJustification {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let justification = match ty {
            BlockType::Macro => BlockJustification::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => BlockJustification::Micro(Deserialize::deserialize(reader)?),
        };
        Ok(justification)
    }
}

/// Struct representing the body of a block.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub fn hash(&self) -> Blake2bHash {
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

impl Serialize for BlockBody {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty().serialize(writer)?;
        size += match self {
            BlockBody::Macro(extrinsics) => extrinsics.serialize(writer)?,
            BlockBody::Micro(extrinsics) => extrinsics.serialize(writer)?,
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty().serialized_size();
        size += match self {
            BlockBody::Macro(extrinsics) => extrinsics.serialized_size(),
            BlockBody::Micro(extrinsics) => extrinsics.serialized_size(),
        };
        size
    }
}

impl Deserialize for BlockBody {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let extrinsics = match ty {
            BlockType::Macro => BlockBody::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => BlockBody::Micro(Deserialize::deserialize(reader)?),
        };
        Ok(extrinsics)
    }
}

// Workaround <https://github.com/bitflags/bitflags/issues/356>
#[allow(clippy::assign_op_pattern)]
mod flags {
    use beserial::{Deserialize, Serialize};
    use bitflags::bitflags;

    #[derive(Default, Serialize, Deserialize)]
    #[repr(transparent)]
    pub struct BlockComponentFlags(u8);

    bitflags! {
        impl BlockComponentFlags: u8 {
            const HEADER  = 0b0000_0001;
            const JUSTIFICATION = 0b0000_0010;
            const BODY = 0b0000_0100;
        }
    }
}
pub use flags::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockComponents {
    pub header: Option<BlockHeader>,
    pub justification: Option<BlockJustification>,
    pub body: Option<BlockBody>,
}

impl BlockComponents {
    pub fn from_block(block: &Block, flags: BlockComponentFlags) -> Self {
        let header = if flags.contains(BlockComponentFlags::HEADER) {
            Some(block.header())
        } else {
            None
        };

        let justification = if flags.contains(BlockComponentFlags::JUSTIFICATION) {
            block.justification()
        } else {
            None
        };

        let body = if flags.contains(BlockComponentFlags::BODY) {
            block.body()
        } else {
            None
        };

        BlockComponents {
            header,
            justification,
            body,
        }
    }
}

impl TryFrom<BlockComponents> for Block {
    type Error = ();

    fn try_from(value: BlockComponents) -> Result<Self, Self::Error> {
        match (value.header, value.justification) {
            (
                Some(BlockHeader::Micro(micro_header)),
                Some(BlockJustification::Micro(micro_justification)),
            ) => {
                let body = value
                    .body
                    .map(|body| {
                        if let BlockBody::Micro(micro_body) = body {
                            Ok(micro_body)
                        } else {
                            Err(())
                        }
                    })
                    .transpose()?;

                Ok(Block::Micro(MicroBlock {
                    header: micro_header,
                    justification: Some(micro_justification),
                    body,
                }))
            }
            (Some(BlockHeader::Macro(macro_header)), macro_justification) => {
                let justification = macro_justification
                    .map(|justification| {
                        if let BlockJustification::Macro(pbft_proof) = justification {
                            Ok(pbft_proof)
                        } else {
                            Err(())
                        }
                    })
                    .transpose()?;

                let body = value
                    .body
                    .map(|body| {
                        if let BlockBody::Macro(macro_body) = body {
                            Ok(macro_body)
                        } else {
                            Err(())
                        }
                    })
                    .transpose()?;

                Ok(Block::Macro(MacroBlock {
                    header: macro_header,
                    justification,
                    body,
                }))
            }
            _ => Err(()),
        }
    }
}
