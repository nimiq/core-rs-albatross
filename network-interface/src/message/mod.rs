use beserial::{uvar, Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_utils::crc::Crc32Computer;
use std::io;
use std::io::Read;

use crate::message::crc::ReaderComputeCrc32;

mod crc;

const MAGIC: u32 = 0x4204_2042;

pub trait Message: Serialize + Deserialize + Send + Sync {
    const TYPE_ID: u64;

    // Does CRC stuff and is called by network
    fn serialize_message<W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = 0;
        let serialized_size: u32 = self.serialized_size() as u32;
        let mut v = Vec::with_capacity(serialized_size as usize);
        size += MAGIC.serialize(&mut v)?;
        size += uvar::from(Self::TYPE_ID).serialize(&mut v)?;
        size += serialized_size.serialize(&mut v)?;
        let checksum_start = v.len();
        size += 0u32.serialize(&mut v)?; // crc32 placeholder

        size += self.serialize(&mut v)?;

        // Write checksum to placeholder.
        let mut v_crc = Vec::with_capacity(4);
        Crc32Computer::default()
            .update(v.as_slice())
            .result()
            .serialize(&mut v_crc)?;

        v[checksum_start..(4 + checksum_start)].clone_from_slice(&v_crc[..4]);

        writer.write_all(v.as_slice())?;
        Ok(size)
    }

    fn deserialize_message<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        // Initialize CRC reader.
        let mut crc32_reader = ReaderComputeCrc32::new(reader);
        let magic: u32 = Deserialize::deserialize(&mut crc32_reader)?;
        if magic != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Wrong magic byte").into());
        }

        // Check for correct type.
        let ty: uvar = Deserialize::deserialize(&mut crc32_reader)?;
        if u64::from(ty) != Self::TYPE_ID {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Wrong message type").into());
        }

        let length: u32 = Deserialize::deserialize(&mut crc32_reader)?;
        // Read checksum.
        crc32_reader.at_checksum = true;
        let checksum: u32 = Deserialize::deserialize(&mut crc32_reader)?;
        crc32_reader.at_checksum = false;
        // Reset length counter.
        crc32_reader.length = 0;

        let message: Self = Deserialize::deserialize(&mut crc32_reader)?;

        if length as usize != crc32_reader.length {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "Incorrect message length").into(),
            );
        }

        // XXX Consume any leftover bytes in the message before computing the checksum.
        // This is consistent with the JS implementation.
        let remaining_length = crc32_reader.read_to_end(&mut Vec::new()).unwrap();
        if remaining_length > 0 {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "Incorrect message length").into(),
            );
        }

        let crc_comp = crc32_reader.crc32.result();
        if crc_comp != checksum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Message deserialization: Bad checksum",
            )
            .into());
        }

        Ok(message)
    }
}

pub trait RequestMessage: Message {
    fn set_request_identifier(&mut self, request_identifier: u32);
}

pub trait ResponseMessage: Message {
    fn get_request_identifier(&self) -> u32;
}
