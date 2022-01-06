//! This module contains an `Encoder` and `Decoder` for the NIMIQ message type. This message type has a fixed header,
//! containing a message type and other auxiliary information. The body of the message can be arbitrary bytes which are
//! later serialized/deserialized to the Message trait.
//!
//! Note that this doesn't actually serialize/deserialize the message content, but only handles reading/writing the
//! message, extracting the type ID and performing consistency checks.
//!

use std::{
    fmt::Debug,
    io::{self, Cursor},
};

use bytes::{Buf, BytesMut};
use thiserror::Error;
use tokio_util::codec::{Decoder, Encoder};

use beserial::{uvar, Deserialize, Serialize, SerializingError};
pub use nimiq_network_interface::message::{Message, MessageType};
use nimiq_network_interface::peer::SendError;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialize(SerializingError),

    #[error("Invalid magic: {0:x}")]
    InvalidMagic(u32),

    #[error("Invalid length: {0}")]
    InvalidLength(u32),
}

impl Error {
    pub fn eof() -> Self {
        Error::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
    }
}

impl From<SerializingError> for Error {
    fn from(e: SerializingError) -> Self {
        match e {
            SerializingError::IoError(e) => Error::Io(e),
            e => Error::Serialize(e),
        }
    }
}

impl From<Error> for SendError {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(e) => SendError::Serialization(e.into()),
            Error::Serialize(e) => SendError::Serialization(e),
            Error::InvalidMagic(_) => SendError::Serialization(SerializingError::InvalidValue),
            Error::InvalidLength(_) => SendError::Serialization(SerializingError::InvalidValue),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub magic: u32, // 0x4204_2042
    pub type_id: uvar,
    pub length: u32,
    pub checksum: u32,
    // data follows from here
}

impl Header {
    pub const MAGIC: u32 = 0x4204_2042;

    fn new(type_id: u64) -> Self {
        Self {
            magic: Self::MAGIC,
            type_id: type_id.into(),
            length: 0,
            checksum: 0,
        }
    }

    fn preliminary_check(&self) -> Result<(), Error> {
        if self.magic != Self::MAGIC {
            Err(Error::InvalidMagic(self.magic))
        } else if self.length < 13 || self.length > 10_000_000 {
            // TODO: I think we should verify that the length is longer than the actual header size (i.e. header.serialized_length())
            Err(Error::InvalidLength(self.length))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
enum DecodeState {
    Head,

    Data {
        header: Header,
        header_length: usize,
    },
}

impl Default for DecodeState {
    fn default() -> Self {
        DecodeState::Head
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageCodec {
    state: DecodeState,
}

impl MessageCodec {
    fn verify(&self, _data: &BytesMut) -> Result<(), Error> {
        // TODO Verify CRC32 checksum
        // Seriously, who had the idea to make the header variable-length with a variable-length field first!
        // We need to skip over the CRC sum when verifying...
        Ok(())
    }
}

impl Decoder for MessageCodec {
    type Item = (MessageType, BytesMut);
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<(MessageType, BytesMut)>, Error> {
        let span = tracing::trace_span!("decode");
        let _enter = span.enter();
        loop {
            match &self.state {
                DecodeState::Head => {
                    // Reserve enough space for the header
                    //
                    // TODO: What's the max size of a header though? I think the max length of the header is 21 bytes.
                    src.reserve(32);

                    // Make a cursor, so we later know how many bytes we read
                    let mut c = Cursor::new(&src);

                    match Header::deserialize(&mut c) {
                        Ok(header) => {
                            // Deserializing the header was successful
                            let header_length = c.position() as usize;

                            // Drop the cursor, so we can mut-borrow the `src` buffer again.
                            drop(c);

                            // Preliminary header check (we can't verify the checksum yet)
                            header.preliminary_check()?;

                            // Reserve enough space
                            src.reserve(header.length as usize);

                            // Set decode state to reading the remaining data
                            self.state = DecodeState::Data {
                                header,
                                header_length,
                            };

                            // Don't return but continue in loop to parse the body.
                        }
                        Err(SerializingError::IoError(e))
                            if matches!(e.kind(), io::ErrorKind::UnexpectedEof) =>
                        {
                            // We just need to wait for more data
                            return Ok(None);
                        }
                        Err(e) => {
                            tracing::warn!("Error decoding message header: {}", e);
                            return Err(e.into());
                        }
                    }
                }
                DecodeState::Data {
                    header,
                    header_length,
                } => {
                    if src.len() >= header.length as usize {
                        // We have read enough bytes to read the full message
                        let message_type = header.type_id.into();

                        // Get buffer of full message
                        let mut data = src.split_to(header.length as usize);

                        // Verify the message (i.e. checksum)
                        self.verify(&data)?;

                        // Skip the header
                        data.advance(*header_length);

                        self.state = DecodeState::Head;

                        return Ok(Some((message_type, data)));
                    } else {
                        // We still need to read more of the message body
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<(MessageType, BytesMut)>, Error> {
        match self.decode(buf) {
            Ok(None) if buf.has_remaining() => Err(Error::eof()),
            r => r,
        }
    }
}

/// Encoder for a full message
impl<M: Message> Encoder<&M> for MessageCodec {
    type Error = Error;

    fn encode(&mut self, message: &M, dst: &mut BytesMut) -> Result<(), Error> {
        let mut header = Header::new(M::TYPE_ID);
        let message_length = header.serialized_size() + message.serialized_size();
        header.length = message_length as u32;

        let existing_length = dst.len();
        dst.reserve(message_length);
        dst.resize(existing_length + message_length, 0);

        let mut c = Cursor::new(dst.as_mut());
        c.set_position(existing_length as u64);

        // Write header
        header.serialize(&mut c)?;

        // Serialize message
        message.serialize(&mut c)?;

        Ok(())
    }
}
