use std::io;

use beserial::{Deserialize, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;
use primitives::block::{Block, BlockBody, Difficulty};
use crate::super_block_counts::SuperBlockCounts;

#[derive(Default, Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ChainInfo {
    pub head: Block,
    pub total_difficulty: Difficulty,
    //pub total_work: (), // TODO: type, BigDecimal? Encapsulated in `Work` type?
    //pub super_block_counts: SuperBlockCounts,
    pub on_main_chain: bool,
    pub main_chain_successor: Option<Blake2bHash>,
    //pub height: u64
}

impl ChainInfo {
    pub fn initial(block: Block) -> Self {
        let total_difficulty = Difficulty::from(block.header.n_bits);
        return ChainInfo {
            head: block,
            total_difficulty,
            //super_block_counts: SuperBlockCounts::zero(),
            on_main_chain: true,
            main_chain_successor: None
        };
    }

    pub fn next(&self, block: Block) -> Self {
        let total_difficulty = &self.total_difficulty + &Difficulty::from(block.header.n_bits);
        return ChainInfo {
            head: block,
            total_difficulty,
            //super_block_counts: self.super_block_counts.clone(), // TODO: get level of block and increment for that depth
            on_main_chain: false,
            main_chain_successor: None
        };
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
        //size += Serialize::serialize(&self.super_block_counts, writer)?;
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
        //size += Serialize::serialized_size(&self.super_block_counts);
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
