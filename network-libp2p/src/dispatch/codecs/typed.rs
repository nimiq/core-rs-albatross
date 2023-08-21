//! This module contains an `Encoder` and `Decoder` for the NIMIQ message type. This message type has a fixed header,
//! containing a message type and other auxiliary information. The body of the message can be arbitrary bytes which are
//! later serialized/deserialized to the Message trait.
//!
//! Note that this doesn't actually serialize/deserialize the message content, but only handles reading/writing the
//! message, extracting the type ID and performing consistency checks.
//!

use std::{fmt::Debug, io};

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{request_response, StreamProtocol};
use unsigned_varint;

/// Maximum request size in bytes (5 kB)
const MAX_REQUEST_SIZE: usize = 5 * 1024;
/// Maximum response size in bytes (10 MB)
const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct MessageCodec {}

pub type IncomingRequest = Vec<u8>;
pub type OutgoingResponse = Vec<u8>;

async fn write_length_prefixed(
    socket: &mut (impl AsyncWrite + Unpin),
    data: impl AsRef<[u8]>,
) -> Result<(), io::Error> {
    write_varint(socket, data.as_ref().len()).await?;
    socket.write_all(data.as_ref()).await?;
    socket.flush().await?;

    Ok(())
}

/// Writes a variable-length integer to the `socket`.
///
/// > **Note**: Does **NOT** flush the socket.
async fn write_varint(socket: &mut (impl AsyncWrite + Unpin), len: usize) -> Result<(), io::Error> {
    let mut len_data = unsigned_varint::encode::usize_buffer();
    let encoded_len = unsigned_varint::encode::usize(len, &mut len_data).len();
    socket.write_all(&len_data[..encoded_len]).await?;

    Ok(())
}

/// Reads a variable-length integer from the `socket`.
///
/// As a special exception, if the `socket` is empty and EOFs right at the beginning, then we
/// return `Ok(0)`.
///
/// > **Note**: This function reads bytes one by one from the `socket`. It is therefore encouraged
/// >           to use some sort of buffering mechanism.
async fn read_varint(socket: &mut (impl AsyncRead + Unpin)) -> Result<usize, io::Error> {
    let mut buffer = unsigned_varint::encode::usize_buffer();
    let mut buffer_len = 0;

    loop {
        match socket.read(&mut buffer[buffer_len..buffer_len + 1]).await? {
            0 => {
                // Reaching EOF before finishing to read the length is an error, unless the EOF is
                // at the very beginning of the substream, in which case we assume that the data is
                // empty.
                if buffer_len == 0 {
                    return Ok(0);
                } else {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
            n => debug_assert_eq!(n, 1),
        }

        buffer_len += 1;

        match unsigned_varint::decode::usize(&buffer[..buffer_len]) {
            Ok((len, _)) => return Ok(len),
            Err(unsigned_varint::decode::Error::Overflow) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "overflow in variable-length integer",
                ));
            }
            Err(_) => {}
        }
    }
}

/// Reads a length-prefixed message from the given socket.
///
/// The `max_size` parameter is the maximum size in bytes of the message that we accept. This is
/// necessary in order to avoid DoS attacks where the remote sends us a message of several
/// gigabytes.
///
/// > **Note**: Assumes that a variable-length prefix indicates the length of the message. This is
/// >           compatible with what [`write_length_prefixed`] does.
async fn read_length_prefixed(
    socket: &mut (impl AsyncRead + Unpin),
    max_size: usize,
) -> io::Result<Vec<u8>> {
    let len = read_varint(socket).await?;
    if len > max_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Received data size ({len} bytes) exceeds maximum ({max_size} bytes)"),
        ));
    }

    let mut buf = vec![0; len];
    socket.read_exact(&mut buf).await?;

    Ok(buf)
}

#[async_trait::async_trait]
impl request_response::Codec for MessageCodec {
    type Protocol = StreamProtocol;
    type Request = IncomingRequest;
    type Response = OutgoingResponse;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let bytes = read_length_prefixed(io, MAX_REQUEST_SIZE).await?;
        Ok(bytes)
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let bytes = read_length_prefixed(io, MAX_RESPONSE_SIZE).await?;
        Ok(bytes)
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
        write_length_prefixed(io, &req).await?;
        io.close().await
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
        write_length_prefixed(io, &res).await?;
        io.close().await
    }
}
