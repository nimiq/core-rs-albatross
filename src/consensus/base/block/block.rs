use beserial::{Deserialize, ReadBytesExt, Serialize, WriteBytesExt};
use std::io;
use super::{BlockBody, BlockHeader, BlockInterlink};

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
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

impl Serialize for Block {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        size += Serialize::serialize(&self.header, writer)?;
        size += self.interlink.serialize(writer, &self.header.prev_hash)?;
        size += Serialize::serialize(&self.body, writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.header);
        size += self.interlink.serialized_size(&self.header.prev_hash);
        size += Serialize::serialized_size(&self.body);
        return size;
    }
}
