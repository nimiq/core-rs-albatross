pub mod crc;

use std::io;

use derive_more::{AsMut, AsRef, Display, From, Into};
use thiserror::Error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

#[derive(
    Copy, Clone, Debug, From, Into, AsRef, AsMut, Display, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct RequestType(u16);

impl RequestType {
    pub const fn new(x: u16) -> Self {
        Self(x)
    }
}

/// Error enumeration for requests
#[derive(Clone, Debug, Error, PartialEq)]
pub enum RequestError {
    /// Outbound request error
    #[error("Outbound error")]
    OutboundRequest(OutboundRequestError),
    /// Inbound request error
    #[error("Inbound error")]
    InboundRequest(InboundRequestError),
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum OutboundRequestError {
    /// The connection closed before a response was received.
    ///
    /// It is not known whether the request may have been
    /// received (and processed) by the remote peer.
    #[error("Connection to peer is closed")]
    ConnectionClosed,
    /// The request could not be sent because a dialing attempt failed.
    #[error("Dial attempt failed")]
    DialFailure,
    /// No receiver was found for this request and no response could be obtained
    #[error("No receiver for request")]
    NoReceiver,
    /// Error sending this request
    #[error("Could't send request")]
    SendError,
    /// Request failed to be serialized
    #[error("Failed to serialized request")]
    SerializationError,
    /// Timeout waiting for the response of this request.
    /// In this case a receiver was registered for responding these requests
    /// but the response never arrived before the timeout was hit.
    #[error("Request timed out")]
    Timeout,
    /// The remote supports none of the requested protocols.
    #[error("Remote doesn't support requested protocol")]
    UnsupportedProtocols,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum InboundRequestError {
    /// Response failed to be deserialized
    #[error("Response failed to be deserialized")]
    DeSerializationError,
    /// Sender future has already been dropped
    #[error("Sender future is already dropped")]
    SenderFutureDropped,
    /// The request timed out before a response could have been sent.
    #[error("Request timed out")]
    Timeout,
}

#[derive(
    Copy, Clone, Debug, From, Into, AsRef, AsMut, Display, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct RequestId(u64);

pub trait Request:
    Serialize + Deserialize + Send + Sync + Unpin + std::fmt::Debug + 'static
{
    const TYPE_ID: u16;

    /// Serializes a request.
    /// A serialized request is composed of:
    /// - A 2 bytes (u16) for the Type ID of the request
    /// - Serialized content of the inner type.
    fn serialize_request<W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = 0;
        let serialized_size = self.serialized_request_size();
        let mut v = Vec::with_capacity(serialized_size);
        size += Self::TYPE_ID.serialize(&mut v)?;
        size += self.serialize(&mut v)?;

        writer.write_all(v.as_slice())?;
        Ok(size)
    }

    /// Computes the size in bytes of a serialized request.
    /// A serialized request is composed of:
    /// - A 2 bytes (u16) for the Type ID of the request
    /// - Serialized content of the inner type.
    fn serialized_request_size(&self) -> usize {
        let mut serialized_size = 0;
        serialized_size += Self::TYPE_ID.serialized_size();
        serialized_size += self.serialized_size();
        serialized_size
    }

    /// Deserializes a request
    /// A serialized request is composed of:
    /// - A 2 bytes (u16) for the Type ID of the request
    /// - Serialized content of the inner type.
    fn deserialize_request<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        // Check for correct type.
        let ty: u16 = Deserialize::deserialize(reader)?;
        if ty != Self::TYPE_ID {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Wrong message type").into());
        }

        let message: Self = Deserialize::deserialize(reader)?;

        Ok(message)
    }
}

pub fn peek_type(buffer: &[u8]) -> Result<RequestType, SerializingError> {
    let ty = u16::deserialize_from_vec(buffer)?;

    Ok(ty.into())
}
