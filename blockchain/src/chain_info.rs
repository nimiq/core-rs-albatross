use std::io;

use beserial::{Deserialize, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;
use primitives::block::{Block, BlockBody, Difficulty, Target};
use crate::SuperBlockCounts;

#[derive(Default, Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ChainInfo {
    pub head: Block,
    pub total_difficulty: Difficulty,
    pub total_work: Difficulty,
    pub super_block_counts: SuperBlockCounts,
    pub on_main_chain: bool,
    pub main_chain_successor: Option<Blake2bHash>,
    //pub height: u64
}

impl ChainInfo {
    // TODO: Does `initial` need a `super_block_counts` parameter?
    pub fn initial(block: Block) -> Self {
        let target = Target::from(&block.header.pow());
        let mut super_block_counts = SuperBlockCounts::default();
        super_block_counts.add(target.get_depth());
        let total_difficulty = Difficulty::from(block.header.n_bits);
        let total_work = Difficulty::from(target);
        ChainInfo {
            head: block,
            total_difficulty,
            total_work,
            super_block_counts,
            on_main_chain: true,
            main_chain_successor: None,
        }
    }

    pub fn next(&self, block: Block) -> Self {
        assert!(self.total_difficulty > Difficulty::from(0u64));
        let target = Target::from(&block.header.pow());
        let super_block_counts = self.super_block_counts.copy_and_add(target.get_depth());
        let total_difficulty = &self.total_difficulty + &Difficulty::from(block.header.n_bits);
        let total_work = &self.total_work + &Difficulty::from(target);
        ChainInfo {
            head: block,
            total_difficulty,
            total_work,
            super_block_counts,
            on_main_chain: false,
            main_chain_successor: None
        }
    }

    pub fn prev(&self, block: Block) -> Self {
        assert!(self.total_difficulty > Difficulty::from(0u64));
        let target = Target::from(&block.header.pow());
        let super_block_counts = self.super_block_counts.copy_and_substract(target.get_depth());
        let total_difficulty = &self.total_difficulty - &Difficulty::from(block.header.n_bits);
        let total_work = &self.total_work - &Difficulty::from(target);
        ChainInfo {
            head: block,
            total_difficulty,
            total_work,
            super_block_counts,
            on_main_chain: false,
            main_chain_successor: None
        }
    }
}

// Do not serialize the block body.
// XXX Move this into Block.serialize_xxx()?
impl Serialize for ChainInfo {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.head.header, writer)?;
        size += Serialize::serialize(&self.head.interlink, writer)?;
        size += Serialize::serialize(&None::<BlockBody>, writer)?;
        size += Serialize::serialize(&self.total_difficulty, writer)?;
        size += Serialize::serialize(&self.total_work, writer)?;
        size += Serialize::serialize(&self.super_block_counts, writer)?;
        size += Serialize::serialize(&self.on_main_chain, writer)?;
        size += Serialize::serialize(&self.main_chain_successor, writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.head.header);
        size += Serialize::serialized_size(&self.head.interlink);
        size += Serialize::serialized_size(&None::<BlockBody>);
        size += Serialize::serialized_size(&self.total_difficulty);
        size += Serialize::serialized_size(&self.total_work);
        size += Serialize::serialized_size(&self.super_block_counts);
        size += Serialize::serialized_size(&self.on_main_chain);
        size += Serialize::serialized_size(&self.main_chain_successor);
        return size;
    }
}

impl IntoDatabaseValue for ChainInfo {
    fn database_byte_size(&self) -> usize {
        return self.serialized_size();
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ChainInfo {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        return Ok(Deserialize::deserialize(&mut cursor)?);
    }
}
