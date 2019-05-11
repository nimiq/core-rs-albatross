use std::io;

use beserial::{Deserialize, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;
use block::{Block, MicroExtrinsics};

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ChainInfo {
    pub head: Block,
    pub on_main_chain: bool,
    pub main_chain_successor: Option<Blake2bHash>,
}

impl ChainInfo {
    pub fn initial(block: Block) -> Self {
        ChainInfo {
            head: block,
            on_main_chain: true,
            main_chain_successor: None,
        }
    }

    pub fn next(&self, block: Block) -> Self {
        ChainInfo {
            head: block,
            on_main_chain: false,
            main_chain_successor: None
        }
    }

    pub fn prev(&self, block: Block) -> Self {
        ChainInfo {
            head: block,
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
        match self.head {
            Block::Macro(ref macro_block) => {
                size += Serialize::serialize(&macro_block.header, writer)?;
                size += Serialize::serialize(&macro_block.justification, writer)?;
            },
            Block::Micro(ref micro_block) => {
                size += Serialize::serialize(&micro_block.header, writer)?;
                size += Serialize::serialize(&micro_block.justification, writer)?;
                size += Serialize::serialize(&None::<MicroExtrinsics>, writer)?;
            }
        }
        size += Serialize::serialize(&self.on_main_chain, writer)?;
        size += Serialize::serialize(&self.main_chain_successor, writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self.head {
            Block::Macro(ref macro_block) => {
                size += Serialize::serialized_size(&macro_block.header);
                size += Serialize::serialized_size(&macro_block.justification);
            },
            Block::Micro(ref micro_block) => {
                size += Serialize::serialized_size(&micro_block.header);
                size += Serialize::serialized_size(&micro_block.justification);
                size += Serialize::serialized_size(&None::<MicroExtrinsics>);
            }
        }
        size += Serialize::serialized_size(&self.on_main_chain);
        size += Serialize::serialized_size(&self.main_chain_successor);
        size
    }
}

impl IntoDatabaseValue for ChainInfo {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ChainInfo {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
