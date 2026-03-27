//! This module contains an `Encoder` and `Decoder` for the NIMIQ request response
//! message type. This message type has a fixed header, containing a message size,
//! and a type ID. The body of the message can be arbitrary bytes which are later
//! serialized/deserialized to the Request/Message trait.
//!
//! Note that this doesn't actually serialize/deserialize the message content, but
//! only handles reading/writing the message.

use std::{io, mem};

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{request_response, StreamProtocol};
use nimiq_network_interface::network;

/// Size of a u64
#[allow(unused_qualifications)] // Remove with a MSVR >= 1.80
const U64_LENGTH: usize = mem::size_of::<u64>();
const MAX_REQUEST_SIZE: u64 = network::MIN_SUPPORTED_REQ_SIZE as u64;
const MAX_RESPONSE_SIZE: u64 = network::MIN_SUPPORTED_RESP_SIZE as u64;

#[derive(Default, Debug, Clone)]
pub struct MessageCodec;

pub type IncomingRequest = Vec<u8>;
pub type OutgoingResponse = Vec<u8>;

async fn read_message<T>(io: &mut T, max_size: u64) -> io::Result<Option<Vec<u8>>>
where
    T: AsyncRead + Unpin + Send,
{
    let mut len_bytes = [0u8; U64_LENGTH];
    match io.read_exact(&mut len_bytes).await {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let len = u64::from_be_bytes(len_bytes);
    if len > max_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Received data size ({len} bytes) exceeds maximum ({max_size} bytes)"),
        ));
    }

    let mut payload = vec![0u8; len as usize];
    match io.read_exact(&mut payload).await {
        Ok(()) => Ok(Some(payload)),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e),
    }
}

#[async_trait::async_trait]
impl request_response::Codec for MessageCodec {
    type Protocol = StreamProtocol;
    type Request = Option<IncomingRequest>;
    type Response = Option<OutgoingResponse>;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_message(io, MAX_REQUEST_SIZE).await
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_message(io, MAX_RESPONSE_SIZE).await
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
        if src.len() as u64 > MAX_REQUEST_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Request data size ({} bytes) exceeds maximum ({MAX_REQUEST_SIZE} bytes)",
                    src.len()
                ),
            ));
        }
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
        if src.len() as u64 > MAX_RESPONSE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Response data size ({} bytes) exceeds maximum ({MAX_RESPONSE_SIZE} bytes)",
                    src.len()
                ),
            ));
        }
        io.write_all(&(src.len() as u64).to_be_bytes()).await?;
        io.write_all(&src).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };

    use futures::{io::Cursor, AsyncRead};
    use libp2p::{request_response::Codec as _, StreamProtocol};
    use nimiq_test_log::test;
    use nimiq_time::timeout;

    use super::MessageCodec;

    struct NoEofReader {
        data: Vec<u8>,
        pos: usize,
    }

    impl NoEofReader {
        fn new(data: Vec<u8>) -> Self {
            Self { data, pos: 0 }
        }
    }

    impl AsyncRead for NoEofReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            if self.pos < self.data.len() {
                let n = (self.data.len() - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                return Poll::Ready(Ok(n));
            }

            Poll::Pending
        }
    }

    #[test(tokio::test)]
    async fn read_request_completes_without_waiting_for_eof() {
        let protocol = StreamProtocol::new("/nimiq/reqres/0.0.1");
        let payload = b"nimiq".to_vec();
        let mut bytes = (payload.len() as u64).to_be_bytes().to_vec();
        bytes.extend_from_slice(&payload);

        let mut reader = NoEofReader::new(bytes);
        let mut codec = MessageCodec;
        let result = timeout(
            Duration::from_millis(100),
            codec.read_request(&protocol, &mut reader),
        )
        .await
        .expect("read_request should not wait for EOF")
        .expect("read_request should succeed");

        assert_eq!(result, Some(payload));
    }

    #[test(tokio::test)]
    async fn read_response_completes_without_waiting_for_eof() {
        let protocol = StreamProtocol::new("/nimiq/reqres/0.0.1");
        let payload = b"albatross".to_vec();
        let mut bytes = (payload.len() as u64).to_be_bytes().to_vec();
        bytes.extend_from_slice(&payload);

        let mut reader = NoEofReader::new(bytes);
        let mut codec = MessageCodec;
        let result = timeout(
            Duration::from_millis(100),
            codec.read_response(&protocol, &mut reader),
        )
        .await
        .expect("read_response should not wait for EOF")
        .expect("read_response should succeed");

        assert_eq!(result, Some(payload));
    }

    #[test(tokio::test)]
    async fn read_request_returns_none_for_truncated_message_with_eof() {
        let protocol = StreamProtocol::new("/nimiq/reqres/0.0.1");
        let mut bytes = (6_u64).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"nim");

        let mut reader = Cursor::new(bytes);
        let mut codec = MessageCodec;
        let result = codec
            .read_request(&protocol, &mut reader)
            .await
            .expect("read_request should not error on truncated EOF");

        assert_eq!(result, None);
    }
}
