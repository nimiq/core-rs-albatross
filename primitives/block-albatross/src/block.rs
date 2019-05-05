use std::fmt;

use crate::micro_block::{MicroBlock, MicroHeader};
use crate::macro_block::{MacroBlock, MacroHeader};
use hash::{Hash, Blake2bHash};
use nimiq_bls::bls12_381::Signature;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use std::cmp::Ordering;
use transaction::Transaction;
use crate::view_change::ViewChangeProof;
use crate::signed::AggregateProof;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockType {
    Macro,
    Micro,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Block {
    Macro(MacroBlock),
    Micro(MicroBlock),
}

impl Block {
    pub fn version(self) -> u16 {
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

    pub fn view_number(&self) -> u16 {
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

    pub fn view_change_proof(&self) -> &Option<ViewChangeProof> {
        match self {
            Block::Macro(ref block) => &block.justification.as_ref().unwrap().view_change_proof, // TODO
            Block::Micro(ref block) => &block.justification.view_change_proof,
        }
    }

    pub fn seed(&self) -> &Signature {
        match self {
            Block::Macro(ref block) => &block.header.seed,
            Block::Micro(ref block) => &block.header.seed,
        }
    }

    pub fn hash(&self) -> Blake2bHash {
        match self {
            Block::Macro(ref block) => block.header.hash(),
            Block::Micro(ref block) => block.header.hash(),
        }
    }

    pub fn header(&self) -> BlockHeader {
        match self {
            Block::Macro(ref block) => BlockHeader::Macro(block.header.clone()),
            Block::Micro(ref block) => BlockHeader::Micro(block.header.clone()),
        }
    }

    pub fn ty(&self) -> BlockType {
        match self {
            Block::Macro(_) => BlockType::Macro,
            Block::Micro(_) => BlockType::Micro
        }
    }
}

impl Serialize for Block {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty().serialize(writer)?;
        size += match self {
            Block::Micro(block) => block.serialize(writer)?,
            Block::Macro(block) => block.serialize(writer)?
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty().serialized_size();
        size += match self {
            Block::Micro(block) => block.serialized_size(),
            Block::Macro(block) => block.serialized_size()
        };
        size
    }
}

impl Deserialize for Block {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let block = match ty {
            BlockType::Macro => Block::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => Block::Macro(Deserialize::deserialize(reader)?)
        };
        Ok(block)
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{}, view {}, type {:?}]", self.block_number(), self.view_number(), self.ty())
    }
}


#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockHeader {
    Micro(MicroHeader),
    Macro(MacroHeader),
}

impl BlockHeader {
    pub fn ty(&self) -> BlockType {
        match self {
            BlockHeader::Micro(_) => BlockType::Micro,
            BlockHeader::Macro(_) => BlockType::Macro
        }
    }
}


impl Serialize for BlockHeader {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty().serialize(writer)?;
        size += match self {
            BlockHeader::Micro(header) => header.serialize(writer)?,
            BlockHeader::Macro(header) => header.serialize(writer)?
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty().serialized_size();
        size += match self {
            BlockHeader::Micro(header) => header.serialized_size(),
            BlockHeader::Macro(header) => header.serialized_size()
        };
        size
    }
}

impl Deserialize for BlockHeader {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let header = match ty {
            BlockType::Macro => BlockHeader::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => BlockHeader::Macro(Deserialize::deserialize(reader)?)
        };
        Ok(header)
    }
}
