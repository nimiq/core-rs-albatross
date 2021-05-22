use std::convert::TryFrom;
use std::fmt;

use bitflags::bitflags;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, SerializeContent};
use nimiq_hash_derive::SerializeContent;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::slots::Validators;
use nimiq_transaction::Transaction;
use nimiq_vrf::VrfSeed;

use crate::macro_block::{MacroBlock, MacroHeader};
use crate::micro_block::{MicroBlock, MicroHeader};
use crate::{MacroBody, MicroBody, MicroJustification, TendermintProof};
use nimiq_primitives::policy;

/// Defines the type of the block, either Micro or Macro (which includes both checkpoint and
/// election blocks).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockType {
    Macro = 1,
    Micro = 2,
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
        policy::epoch_at(self.block_number())
    }

    /// Returns the view number of the block.
    pub fn view_number(&self) -> u32 {
        match self {
            Block::Macro(ref block) => block.header.view_number,
            Block::Micro(ref block) => block.header.view_number,
        }
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

    /// Returns the next view number, assuming that there was no view change. This will return 0, if
    /// the next block is the first of the batch (i.e. the current one is a macro block), or the
    /// view number of the current block.
    /// To get the view number for after a single view change, just add 1.
    pub fn next_view_number(&self) -> u32 {
        match self {
            // If the previous block was a macro block, this resets the view number.
            Block::Macro(_) => 0,
            // Otherwise we are now at the view number of the previous block.
            Block::Micro(ref block) => block.header.view_number,
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
        Some(match self {
            Block::Macro(ref block) => BlockBody::Macro(block.body.as_ref()?.clone()),
            Block::Micro(ref block) => BlockBody::Micro(block.body.as_ref()?.clone()),
        })
    }

    /// Returns a reference to the transactions of the block. If the block is a Macro block it just
    /// returns None, since Macro blocks don't contain any transactions.
    pub fn transactions(&self) -> Option<&Vec<Transaction>> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref block) => block.body.as_ref().map(|ex| &ex.transactions),
        }
    }

    /// Returns a mutable reference to the transactions of the block. If the block is a Macro block
    /// it just returns None, since Macro blocks don't contain any transactions.
    pub fn transactions_mut(&mut self) -> Option<&mut Vec<Transaction>> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref mut block) => block.body.as_mut().map(|ex| &mut ex.transactions),
        }
    }

    /// Returns the sum of the fees of all of the transactions in the block. If the block is a Macro
    /// block it just returns zero, since Macro blocks don't contain any transactions.
    pub fn sum_transaction_fees(&self) -> Coin {
        match self {
            Block::Macro(_) => Coin::ZERO,
            Block::Micro(ref block) => block
                .body
                .as_ref()
                .map(|ex| ex.transactions.iter().map(|tx| tx.fee).sum())
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
    pub fn unwrap_transactions(self) -> Vec<Transaction> {
        self.unwrap_micro().body.unwrap().transactions
    }

    /// Returns true if the block is a Micro block, false otherwise.
    pub fn is_micro(&self) -> bool {
        matches!(self, Block::Micro(_))
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
        write!(
            f,
            "[#{}, view {}, type {:?}]",
            self.block_number(),
            self.view_number(),
            self.ty()
        )
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

    /// Returns the view number of the block.
    pub fn view_number(&self) -> u32 {
        match self {
            BlockHeader::Macro(ref header) => header.view_number,
            BlockHeader::Micro(ref header) => header.view_number,
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

    /// Returns the next view number, assuming that there was no view change. This will return 0, if
    /// the next block is the first of the batch (i.e. the current one is a macro block), or the
    /// view number of the current block.
    /// To get the view number for after a single view change, just add 1.
    pub fn next_view_number(&self) -> u32 {
        match self {
            // If the previous block was a macro block, this resets the view number.
            BlockHeader::Macro(_) => 0,
            // Otherwise we are now at the view number of the previous block.
            BlockHeader::Micro(header) => header.view_number,
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
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
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

bitflags! {
    #[derive(Default, Serialize, Deserialize)]
    pub struct BlockComponentFlags: u8 {
        const HEADER  = 0b0000_0001;
        const JUSTIFICATION = 0b0000_0010;
        const BODY = 0b0000_0100;
    }
}

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
