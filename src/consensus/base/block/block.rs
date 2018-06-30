use beserial::{Deserialize, ReadBytesExt, Serialize};
use std::io;
use super::{BlockBody, BlockHeader, BlockInterlink};

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize)]
pub struct Block {
    pub header: BlockHeader,
    pub interlink: BlockInterlink,
    pub body: Option<BlockBody>,
}

impl Deserialize for Block {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let header: BlockHeader = Deserialize::deserialize(reader)?;
        let interlink = BlockInterlink::deserialize(reader, &header.prev_hash)?;
        return Ok(Block {
            header,
            interlink,
            body: Deserialize::deserialize(reader)?,
        });
    }
}
