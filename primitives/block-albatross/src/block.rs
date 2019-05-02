use std::fmt;

use crate::micro_block::{MicroBlock, MicroHeader};
use crate::macro_block::{MacroBlock, MacroHeader};
use hash::{Hash, Blake2bHash};
use keys::Signature;

pub type Seed = Hash;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
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
    pub fn block_number(&self) -> u32 {
        match self {
            Block::Macro(ref block) => block.header.digest.block_number,
            Block::Micro(ref block) => block.header.digest.block_number,
        }
    }

    pub fn view_number(&self) -> u16 {
        match self {
            Block::Macro(ref block) => block.header.digest.view_number,
            Block::Micro(ref block) => block.header.digest.view_number,
        }
    }

    pub fn block_type(&self) -> BlockType {
        match self {
            Block::Macro(_) => BlockType::Macro,
            Block::Micro(_) => BlockType::Micro,
        }
    }

    pub fn seed(&self) -> u64 {
        match self {
            Block::Macro(ref block) => block.extrinsics.seed,
            Block::Micro(ref block) => block.extrinsics.seed,
        }
    }

    pub fn hash(&self) -> Blake2bHash {
        match self {
            Block::Macro(ref block) => block.header.hash(),
            Block::Micro(ref block) => block.header.hash(),
        }
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{}, view {}, type {:?}]", self.block_number(), self.view_number(), self.block_type())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockHeader {
    Micro(MicroHeader),
    Macro(MacroHeader),
}

impl BlockHeader {
    pub fn hash(&self) -> Blake2bHash {
        match self {
            BlockHeader::Micro(ref header) => header.hash(),
            BlockHeader::Macro(ref header) => header.hash(),
        }
    }
}
