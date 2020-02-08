use std::fmt;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use block_base;
use hash::{Blake2bHash, Hash, SerializeContent};
use hash_derive::SerializeContent;
use primitives::networks::NetworkId;
use transaction::Transaction;
use vrf::VrfSeed;

use crate::BlockError;
use crate::macro_block::{MacroBlock, MacroHeader};
use crate::micro_block::{MicroBlock, MicroHeader};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockType {
    Macro = 1,
    Micro = 2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Block {
    Macro(MacroBlock),
    Micro(MicroBlock),
}

impl Block {
    pub const VERSION: u16 = 1;

    pub fn verify(&self, network_id: NetworkId) -> Result<(), BlockError> {
        match self {
            Block::Macro(ref block) => block.verify(),
            Block::Micro(ref block) => block.verify(network_id),
        }
    }

    pub fn version(&self) -> u16 {
        match self {
            Block::Macro(ref block) => block.header.version,
            Block::Micro(ref block) => block.header.version,
        }
    }

    pub fn block_number(&self) -> u32 {
        match self {
            Block::Macro(ref block) => block.header.block_number,
            Block::Micro(ref block) => block.header.block_number,
        }
    }

    pub fn view_number(&self) -> u32 {
        match self {
            Block::Macro(ref block) => block.header.view_number,
            Block::Micro(ref block) => block.header.view_number,
        }
    }

    /// The next view number, if there is no view change. This will return 0, if the next block
    /// will be the first of the epoch (i.e. the current one is a macro block), or the view number
    /// of the current block.
    /// To get the view number for after a single view change, just add 1
    pub fn next_view_number(&self) -> u32 {
        match self {
            // If the previous block was a macro block, this resets the view number
            Block::Macro(_) => 0,
            // Otherwise we are now at the view number of the previous block
            Block::Micro(ref block) => block.header.view_number
        }
    }

    pub fn parent_hash(&self) -> &Blake2bHash {
        match self {
            Block::Macro(ref block) => &block.header.parent_hash,
            Block::Micro(ref block) => &block.header.parent_hash,
        }
    }

    pub fn timestamp(&self) -> u64 {
        match self {
            Block::Macro(ref block) => block.header.timestamp,
            Block::Micro(ref block) => block.header.timestamp,
        }
    }

    pub fn seed(&self) -> &VrfSeed {
        match self {
            Block::Macro(ref block) => &block.header.seed,
            Block::Micro(ref block) => &block.header.seed,
        }
    }

    pub fn state_root(&self) -> &Blake2bHash {
        match self {
            Block::Macro(ref block) => &block.header.state_root,
            Block::Micro(ref block) => &block.header.state_root,
        }
    }

    pub fn hash(&self) -> Blake2bHash {
        match self {
            Block::Macro(ref block) => block.header.hash(),
            Block::Micro(ref block) => block.header.hash(),
        }
    }

    pub fn header(&self) -> BlockHeader {
        // TODO Can we eliminate the clone()s here?
        match self {
            Block::Macro(ref block) => BlockHeader::Macro(block.header.clone()),
            Block::Micro(ref block) => BlockHeader::Micro(block.header.clone()),
        }
    }

    pub fn transactions(&self) -> Option<&Vec<Transaction>> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref block) => block.extrinsics.as_ref().map(|ex| &ex.transactions)
        }
    }

    pub fn transactions_mut(&mut self) -> Option<&mut Vec<Transaction>> {
        match self {
            Block::Macro(_) => None,
            Block::Micro(ref mut block) => block.extrinsics.as_mut().map(|ex| &mut ex.transactions)
        }
    }

    pub fn unwrap_macro_ref(&self) -> &MacroBlock {
        if let Block::Macro(ref block) = self {
            block
        } else {
            unreachable!()
        }
    }

    pub fn unwrap_micro_ref(&self) -> &MicroBlock {
        if let Block::Micro(ref block) = self {
            block
        } else {
            unreachable!()
        }
    }

    pub fn unwrap_macro(self) -> MacroBlock {
        if let Block::Macro(block) = self {
            block
        } else {
            unreachable!()
        }
    }

    pub fn unwrap_micro(self) -> MicroBlock {
        if let Block::Micro(block) = self {
            block
        } else {
            unreachable!()
        }
    }

    pub fn ty(&self) -> BlockType {
        match self {
            Block::Macro(_) => BlockType::Macro,
            Block::Micro(_) => BlockType::Micro,
        }
    }

    pub fn unwrap_transactions(self) -> Vec<Transaction> {
        self.unwrap_micro().extrinsics.unwrap().transactions
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
        write!(f, "[#{}, view {}, type {:?}]", self.block_number(), self.view_number(), self.ty())
    }
}


#[derive(Clone, Debug, Eq, PartialEq, SerializeContent)]
pub enum BlockHeader {
    Micro(MicroHeader),
    Macro(MacroHeader),
}

impl BlockHeader {
    pub fn ty(&self) -> BlockType {
        match self {
            BlockHeader::Macro(_) => BlockType::Macro,
            BlockHeader::Micro(_) => BlockType::Micro,
        }
    }

    pub fn block_number(&self) -> u32 {
        match self {
            BlockHeader::Macro(ref header) => header.block_number,
            BlockHeader::Micro(ref header) => header.block_number,
        }
    }

    pub fn view_number(&self) -> u32 {
        match self {
            BlockHeader::Macro(ref header) => header.view_number,
            BlockHeader::Micro(ref header) => header.view_number,
        }
    }

    pub fn seed(&self) -> &VrfSeed {
        match self {
            BlockHeader::Macro(ref header) => &header.seed,
            BlockHeader::Micro(ref header) => &header.seed,
        }
    }

    pub fn hash(&self) -> Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => header.hash(),
            BlockHeader::Micro(ref header) => header.hash(),
        }
    }

    pub fn parent_hash(&self) -> &Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => &header.parent_hash,
            BlockHeader::Micro(ref header) => &header.parent_hash
        }
    }

    /// The next view number, if there is no view change. This will return 0, if the next block
    /// will be the first of the epoch (i.e. the current one is a macro block), or the view number
    /// of the current block.
    /// To get the view number for after a single view change, just add 1
    pub fn next_view_number(&self) -> u32 {
        match self {
            // If the previous block was a macro block, this resets the view number
            BlockHeader::Macro(_) => 0,
            // Otherwise we are now at the view number of the previous block
            BlockHeader::Micro(header) => header.view_number
        }
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for BlockHeader {}

impl block_base::BlockHeader for BlockHeader {
    fn hash(&self) -> Blake2bHash {
        BlockHeader::hash(self)
    }

    fn height(&self) -> u32 {
        self.block_number()
    }

    fn timestamp(&self) -> u64 {
        match self {
            BlockHeader::Macro(header) => header.timestamp,
            BlockHeader::Micro(header) => header.timestamp,
        }
    }
}

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

impl block_base::Block for Block {
    type Header = BlockHeader;
    type Error = BlockError;

    fn hash(&self) -> Blake2bHash {
        self.hash()
    }

    fn prev_hash(&self) -> &Blake2bHash {
        self.parent_hash()
    }

    fn height(&self) -> u32 {
        self.block_number()
    }

    fn header(&self) -> Self::Header {
        self.header()
    }

    fn transactions(&self) -> Option<&Vec<Transaction>> {
        self.transactions()
    }

    fn transactions_mut(&mut self) -> Option<&mut Vec<Transaction>> {
        self.transactions_mut()
    }
}

