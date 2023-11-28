//! This module contains an `Encoder` and `Decoder` for the NIMIQ request response
//! message type. This message type has a fixed header, containing a message size,
//! and a type ID. The body of the message can be arbitrary bytes which are later
//! serialized/deserialized to the Request/Message trait.
//!
//! Note that this doesn't actually serialize/deserialize the message content, but
//! only handles reading/writing the message.

use std::io;

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{request_response, StreamProtocol};

/// Maximum request size in bytes (5 kB)
const MAX_REQUEST_SIZE: u64 = 5 * 1024;
/// Maximum response size in bytes (10 MB)
const MAX_RESPONSE_SIZE: u64 = 10 * 1024 * 1024;
/// Size of a u64
const U64_LENGTH: usize = std::mem::size_of::<u64>();

#[derive(Default, Debug, Clone)]
pub struct MessageCodec;

pub type IncomingRequest = Vec<u8>;
pub type OutgoingResponse = Vec<u8>;

#[async_trait::async_trait]
impl request_response::Codec for MessageCodec {
    type Protocol = StreamProtocol;
    type Request = Option<IncomingRequest>;
    type Response = Option<OutgoingResponse>;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut vec = Vec::new();
        io.take(MAX_REQUEST_SIZE).read_to_end(&mut vec).await?;
        if vec.len() < U64_LENGTH {
            return Ok(None);
        }
        let mut len_bytes = [0u8; U64_LENGTH];
        len_bytes.copy_from_slice(&vec[..U64_LENGTH]);
        let len = u64::from_be_bytes(len_bytes) as usize;

        if len as u64 > MAX_REQUEST_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Received data size ({len} bytes) exceeds maximum ({MAX_REQUEST_SIZE} bytes)"
                ),
            ));
        }

        if vec.len() - U64_LENGTH >= len {
            // Skip the length header we already read
            vec.drain(..U64_LENGTH);
            Ok(Some(vec))
        } else {
            Ok(None)
        }
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut vec = Vec::new();
        io.take(MAX_RESPONSE_SIZE).read_to_end(&mut vec).await?;
        if vec.len() < U64_LENGTH {
            return Ok(None);
        }
        let mut len_bytes = [0u8; U64_LENGTH];
        len_bytes.copy_from_slice(&vec[..U64_LENGTH]);
        let len = u64::from_be_bytes(len_bytes) as usize;

        if len as u64 > MAX_RESPONSE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Received data size ({len} bytes) exceeds maximum ({MAX_RESPONSE_SIZE} bytes)"
                ),
            ));
        }

        if vec.len() - U64_LENGTH >= len {
            // Skip the length header we already read
            vec.drain(..U64_LENGTH);
            Ok(Some(vec))
        } else {
            Ok(None)
        }
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Send + Unpin,
    {
        let src = req.expect("No data to write");
        io.write_all(&(src.len() as u64).to_be_bytes()).await?;
        io.write_all(&src).await?;
        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let src = res.expect("No data to write");
        io.write_all(&(src.len() as u64).to_be_bytes()).await?;
        io.write_all(&src).await?;
        Ok(())
    }
}
