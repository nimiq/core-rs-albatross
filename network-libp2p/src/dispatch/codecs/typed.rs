//! This module contains an `Encoder` and `Decoder` for the NIMIQ message type. This message type has a fixed header,
//! containing a message type and other auxiliary information. The body of the message can be arbitrary bytes which are
//! later serialized/deserialized to the Message trait.
//!
//! Note that this doesn't actually serialize/deserialize the message content, but only handles reading/writing the
//! message, extracting the type ID and performing consistency checks.
//!

use std::fmt::Debug;
use std::io;

use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};
use libp2p::core::{upgrade, ProtocolName};
use libp2p::request_response::RequestResponseCodec;

use crate::REQRES_PROTOCOL;

/// Maximum request size in bytes (5 kB)
const MAX_REQUEST_SIZE: usize = 5 * 1024;
/// Maximum response size in bytes (10 MB)
const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct MessageCodec {}

pub type IncomingRequest = Vec<u8>;
pub type OutgoingResponse = Vec<u8>;

#[derive(Debug, Clone)]
pub enum ReqResProtocol {
    Version1,
}

impl ProtocolName for ReqResProtocol {
    fn protocol_name(&self) -> &[u8] {
        match *self {
            ReqResProtocol::Version1 => REQRES_PROTOCOL,
        }
    }
}

#[async_trait::async_trait]
impl RequestResponseCodec for MessageCodec {
    type Protocol = ReqResProtocol;
    type Request = IncomingRequest;
    type Response = OutgoingResponse;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let bytes = upgrade::read_length_prefixed(io, MAX_REQUEST_SIZE).await?;
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
        let bytes = upgrade::read_length_prefixed(io, MAX_RESPONSE_SIZE).await?;
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
        upgrade::write_length_prefixed(io, &req).await?;
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
        upgrade::write_length_prefixed(io, &res).await?;
        io.close().await
    }
}
