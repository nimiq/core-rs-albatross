use std::io;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use block::{Block, BlockType, MacroExtrinsics, MicroExtrinsics};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;


#[derive(Clone, PartialEq, Eq, Debug)]
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

    pub fn new(block: Block) -> Self {
        ChainInfo {
            head: block,
            on_main_chain: false,
            main_chain_successor: None,
        }
    }
}

// Do not serialize the block body.
// XXX Move this into Block.serialize_xxx()?
impl Serialize for ChainInfo {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.head.ty(), writer)?;
        match self.head {
            Block::Macro(ref macro_block) => {
                size += Serialize::serialize(&macro_block.header, writer)?;
                size += Serialize::serialize(&macro_block.justification, writer)?;
                size += Serialize::serialize(&None::<MacroExtrinsics>, writer)?;
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
        size += Serialize::serialized_size(&self.head.ty());
        match self.head {
            Block::Macro(ref macro_block) => {
                size += Serialize::serialized_size(&macro_block.header);
                size += Serialize::serialized_size(&macro_block.justification);
                size += Serialize::serialized_size(&None::<MacroExtrinsics>);
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

impl Deserialize for ChainInfo {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let head = match ty {
            BlockType::Macro => Block::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => Block::Micro(Deserialize::deserialize(reader)?),
        };
        let on_main_chain = Deserialize::deserialize(reader)?;
        let main_chain_successor = Deserialize::deserialize(reader)?;

        Ok(ChainInfo {
            head,
            on_main_chain,
            main_chain_successor,
        })
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
