use beserial::{Serialize, Deserialize, WriteBytesExt};
use crate::consensus::base::block::{Block, BlockBody, Difficulty};
use crate::consensus::base::primitive::hash::Blake2bHash;
use crate::utils::db::{FromDatabaseValue, IntoDatabaseValue};
use std::io;

#[derive(Default, Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ChainData {
    pub head: Block,
    pub total_difficulty: Difficulty,
    pub on_main_chain: bool,
    pub main_chain_successor: Option<Blake2bHash>
}

impl ChainData {
    pub fn initial(block: Block) -> Self {
        let total_difficulty = Difficulty::from(block.header.n_bits);
        return ChainData {
            head: block,
            total_difficulty,
            on_main_chain: true,
            main_chain_successor: None
        };
    }

    pub fn next(&self, block: Block) -> Self {
        let total_difficulty = &self.total_difficulty + &Difficulty::from(block.header.n_bits);
        return ChainData {
            head: block,
            total_difficulty,
            on_main_chain: false,
            main_chain_successor: None
        };
    }
}

// Do not serialize the block body.
// XXX Move this into Block.serialize_xxx()?
impl Serialize for ChainData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        size += Serialize::serialize(&self.head.header, writer)?;
        size += Serialize::serialize(&self.head.interlink, writer)?;
        size += Serialize::serialize(&None::<BlockBody>, writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.head.header);
        size += Serialize::serialized_size(&self.head.interlink);
        size += Serialize::serialized_size(&None::<BlockBody>);
        return size;
    }
}

impl IntoDatabaseValue for ChainData {
    fn database_byte_size(&self) -> usize {
        return self.serialized_size();
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ChainData {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        return Deserialize::deserialize(&mut cursor);
    }
}
