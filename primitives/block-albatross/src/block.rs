use std::fmt;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use block_base;
use hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_bls::bls12_381::CompressedSignature;
use primitives::networks::NetworkId;
use transaction::Transaction;

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

    pub fn seed(&self) -> &CompressedSignature {
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

    pub fn ty(&self) -> BlockType {
        match self {
            Block::Macro(_) => BlockType::Macro,
            Block::Micro(_) => BlockType::Micro,
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

    pub fn hash(&self) -> Blake2bHash {
        match self {
            BlockHeader::Macro(ref header) => header.hash(),
            BlockHeader::Micro(ref header) => header.hash(),
        }
    }
}

impl Hash for BlockHeader {}

impl block_base::BlockHeader for BlockHeader {
    fn hash(&self) -> Blake2bHash {
        BlockHeader::hash(self)
    }

    fn height(&self) -> u32 {
        self.block_number()
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

