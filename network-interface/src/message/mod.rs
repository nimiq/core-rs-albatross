use std::io;
use std::io::{Cursor, Read, Seek, SeekFrom};

use derive_more::{From, Into, AsRef, AsMut, Display};

use beserial::{uvar, Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use futures::{AsyncRead, AsyncReadExt};
use nimiq_utils::crc::Crc32Computer;

use crate::message::crc::ReaderComputeCrc32;

mod crc;


#[derive(Copy, Clone, Debug, From, Into, AsRef, AsMut, Display, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageType(u64);

impl MessageType {
    pub const fn new(x: u64) -> Self {
        Self(x)
    }
}

impl From<uvar> for MessageType {
    fn from(x: uvar) -> Self {
        Self(x.into())
    }
}

impl From<MessageType> for uvar {
    fn from(x: MessageType) -> Self {
        x.0.into()
    }
}


const MAGIC: u32 = 0x4204_2042;

pub trait Message: Serialize + Deserialize + Send + Sync + Unpin + std::fmt::Debug + 'static {
    const TYPE_ID: u64;

    // Does CRC stuff and is called by network
    fn serialize_message<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        let ty = uvar::from(Self::TYPE_ID);
        let serialized_size = self.serialized_message_size() as u32;

        let mut v = Vec::with_capacity(serialized_size as usize);
        size += MAGIC.serialize(&mut v)?;
        size += ty.serialize(&mut v)?;
        size += serialized_size.serialize(&mut v)?;
        let checksum_start = v.len();
        size += 0u32.serialize(&mut v)?; // crc32 placeholder

        size += self.serialize(&mut v)?;

        // Write checksum to placeholder.
        let mut v_crc = Vec::with_capacity(4);
        Crc32Computer::default().update(v.as_slice()).result().serialize(&mut v_crc)?;

        v[checksum_start..(4 + checksum_start)].clone_from_slice(&v_crc[..4]);

        writer.write_all(v.as_slice())?;
        Ok(size)
    }

    fn serialized_message_size(&self) -> usize {
        let mut serialized_size = 4 + 4 + 4; // magic + serialized_size + checksum
        serialized_size += uvar::from(Self::TYPE_ID).serialized_size();
        serialized_size += self.serialized_size();
        serialized_size
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

        let message: Self = Deserialize::deserialize(&mut crc32_reader)?;

        if length as usize != crc32_reader.length {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Incorrect message length").into());
        }

        // XXX Consume any leftover bytes in the message before computing the checksum.
        // This is consistent with the JS implementation.
        let remaining_length = crc32_reader.read_to_end(&mut Vec::new()).unwrap();
        if remaining_length > 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Incorrect message length").into());
        }

        let crc_comp = crc32_reader.crc32.result();
        if crc_comp != checksum {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Message deserialization: Bad checksum").into());
        }

        Ok(message)
    }
}

pub fn peek_type(buffer: &[u8]) -> Result<u64, SerializingError> {
    let mut c = Cursor::new(buffer);

    // skip 4 bytes of magic
    c.seek(SeekFrom::Start(4))?;

    let ty = uvar::deserialize(&mut c)?;

    Ok(u64::from(ty))
}

pub fn peek_length(buffer: &[u8]) -> Result<usize, SerializingError> {
    let mut c = Cursor::new(buffer);

    // skip 4 bytes of magic
    c.seek(SeekFrom::Start(4))?;

    // skip type (uvar)
    let _ = uvar::deserialize(&mut c)?;
    let n = u32::deserialize(&mut c)?;

    Ok(n as usize)
}

pub async fn read_message<R: AsyncRead + Unpin>(mut reader: R) -> Result<Vec<u8>, SerializingError> {
    log::trace!("read_message: reading magic and first byte of type...");
    // Read message magic and first type byte.
    let mut msg = vec![0; 5];
    reader.read_exact(&mut msg).await?;

    // Read type remainder and message length.
    let header_len = uvar::serialized_size_from_first_byte(msg[4]) + 8;
    msg.resize(header_len, 0);
    reader.read_exact(&mut msg[5..]).await?;

    log::trace!("read_message: header: {}", pretty_hex::pretty_hex(&msg));

    // Check message size.
    let msg_len = peek_length(&msg[..])?;
    if msg_len < 13 {
        return Err(SerializingError::InvalidValue);
    } else if msg_len > 10_000_000 {
        return Err(SerializingError::LimitExceeded);
    }

    log::trace!("read_message: msg_len = {}", msg_len);

    // Read remainder of message.
    // FIXME Don't allocate the whole message buffer immediately.
    // TODO Copy message in chunks and grow incrementally.
    msg.resize(msg_len, 0);

    log::trace!("read_message: reading remainder of message");
    reader.read_exact(&mut msg[header_len..]).await?;

    log::trace!("read_message: finished reading message: {:?}", msg);

    Ok(msg)
}

pub trait RequestMessage: Message {
    fn set_request_identifier(&mut self, request_identifier: u32);
}

pub trait ResponseMessage: Message {
    fn get_request_identifier(&self) -> u32;
}
