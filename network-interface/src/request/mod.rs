use std::{fmt, sync::Arc, time::Duration};

use futures::{stream::BoxStream, Future, StreamExt};
use nimiq_serde::{Deserialize, DeserializeError, Serialize, SerializedMaxSize};
use thiserror::Error;

/// The range to restrict the responses to the requests on the network layer.
pub const DEFAULT_MAX_REQUEST_RESPONSE_TIME_WINDOW: Duration = Duration::from_secs(10);

use crate::network::Network;

#[derive(Copy, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RequestType(pub u16);

impl fmt::Display for RequestType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.type_id(),
            if self.requires_response() { 'r' } else { 'm' },
        )
    }
}

impl RequestType {
    const fn new(type_id: u16, requires_response: bool) -> Self {
        Self((type_id << 1) | requires_response as u16)
    }
    pub fn from_request<R: RequestCommon>() -> Self {
        Self::new(R::TYPE_ID, R::Kind::EXPECT_RESPONSE)
    }
    pub const fn request(type_id: u16) -> Self {
        Self::new(type_id, true)
    }
    pub const fn message(type_id: u16) -> Self {
        Self::new(type_id, false)
    }
    pub const fn type_id(self) -> u16 {
        self.0 >> 1
    }
    pub const fn requires_response(self) -> bool {
        self.0 & 1 != 0
    }
}

/// Error enumeration for requests
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RequestError {
    /// Outbound request error
    #[error("Outbound error: {0}")]
    OutboundRequest(#[from] OutboundRequestError),
    /// Inbound request error
    #[error("Inbound error: {0}")]
    InboundRequest(#[from] InboundRequestError),
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
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
    /// No receiver was found for this request and no response could be transmitted
    #[error("No receiver for request")]
    NoReceiver,
    /// Error sending this request
    #[error("Couldn't send request")]
    SendError,
    /// Sender future has already been dropped
    #[error("Sender future is already dropped")]
    SenderFutureDropped,
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
    /// No response after asking a couple of peers.
    #[error("No response after asking a couple of peers")]
    NoResponse,
    /// Error that doesn't match any of the other error causes
    #[error("Other: {0}")]
    Other(String),
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq, Serialize, Deserialize)]
pub enum InboundRequestError {
    /// Response failed to be deserialized
    #[error("Response failed to be deserialized")]
    DeSerializationError = 1,
    /// No receiver was found for this incoming request
    #[error("No receiver for request")]
    NoReceiver = 2,
    /// Sender future has already been dropped
    #[error("Sender future is already dropped")]
    SenderFutureDropped = 3,
    /// The request timed out before a response could have been sent.
    #[error("Request timed out")]
    Timeout = 4,
    /// The request exceeded the maximum defined rate limit for its request type.
    #[error("Request exceeds the maximum rate limit")]
    ExceedsRateLimit = 5,
}

pub trait RequestKind {
    const EXPECT_RESPONSE: bool;
}

pub struct RequestMarker;
pub struct MessageMarker;

impl RequestKind for RequestMarker {
    const EXPECT_RESPONSE: bool = true;
}
impl RequestKind for MessageMarker {
    const EXPECT_RESPONSE: bool = false;
}

pub trait RequestCommon:
    Serialize + Deserialize + Send + Sync + Unpin + fmt::Debug + 'static
{
    type Kind: RequestKind;
    const TYPE_ID: u16;
    type Response: Deserialize + Serialize + SerializedMaxSize + Send;
    const MAX_REQUESTS: u32;
    const TIME_WINDOW: Duration = DEFAULT_MAX_REQUEST_RESPONSE_TIME_WINDOW;

    /// Returns the type name of the given request type `T`.
    /// This only works for
    ///   - non-generic types
    ///   - generic types with a single level of nesting, in which case the name of
    ///     the type parameter is returned
    fn type_name<T>() -> &'static str {
        let name = std::any::type_name::<T>();
        let name = match name.rfind("::") {
            Some(index) => &name[index + 2..],
            None => name,
        };
        match name.rfind(">") {
            Some(index) => &name[..index],
            None => name,
        }
    }
}

pub trait RequestSerialize: RequestCommon {
    /// Serializes a request.
    /// A serialized request is composed of:
    /// - A variable sized integer for the Type ID of the request
    /// - Serialized content of the inner type.
    fn serialize_request(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.serialized_request_size());
        RequestType::from_request::<Self>()
            .serialize_to_writer(&mut data)
            .unwrap();
        Serialize::serialize_to_writer(self, &mut data).unwrap();
        data
    }

    /// Computes the size in bytes of a serialized request.
    /// A serialized request is composed of:
    /// - A 2 bytes (u16) for the Type ID of the request
    /// - Serialized content of the inner type.
    fn serialized_request_size(&self) -> usize {
        let mut size = 0;
        size += RequestType::from_request::<Self>().0.serialized_size();
        size += self.serialized_size();
        size
    }

    /// Deserializes a request
    /// A serialized request is composed of:
    /// - A variable sized integer for the Type ID of the request
    /// - Serialized content of the inner type.
    fn deserialize_request(buffer: &[u8]) -> Result<Self, DeserializeError> {
        // Check for correct type.
        let (ty, message_buf) = u16::deserialize_take(buffer)?;
        if ty != RequestType::from_request::<Self>().0 {
            return Err(DeserializeError::bad_enum());
        }
        Self::deserialize_from_vec(message_buf)
    }
}

impl<T: RequestCommon> RequestSerialize for T {}

pub trait Request: RequestCommon<Kind = RequestMarker> {}
pub trait Message: RequestCommon<Kind = MessageMarker, Response = ()> {}

impl<T: RequestCommon<Kind = RequestMarker>> Request for T {}
impl<T: RequestCommon<Kind = MessageMarker, Response = ()>> Message for T {}

pub fn peek_type(buffer: &[u8]) -> Result<RequestType, DeserializeError> {
    let ty = u16::deserialize_from_vec(buffer)?;
    Ok(RequestType(ty))
}

/// This trait defines the behaviour when receiving a message and how to generate the response.
pub trait Handle<N: Network, T>: Request {
    fn handle(&self, peer_id: N::PeerId, context: &T) -> <Self as RequestCommon>::Response;
}

/// This trait defines the behaviour when receiving a message
pub trait MessageHandle<N: Network, T> {
    fn message_handle(&self, peer_id: N::PeerId, context: &T);
}

const MAX_CONCURRENT_HANDLERS: usize = 64;

pub fn request_handler<T: Send + Sync + Clone + 'static, Req: Handle<N, T>, N: Network>(
    network: &Arc<N>,
    stream: BoxStream<'static, (Req, N::RequestId, N::PeerId)>,
    req_environment: &T,
) -> impl Future<Output = ()> {
    let req_environment = req_environment.clone();
    let network = Arc::clone(network);
    async move {
        stream
            .for_each_concurrent(MAX_CONCURRENT_HANDLERS, |(msg, request_id, peer_id)| {
                let network = Arc::clone(&network);
                let req_environment = req_environment.clone();
                async move {
                    // Try to send the response. If it fails, it usually means that the peer has
                    // disconnected, so we silently ignore any errors here.
                    network
                        .respond::<Req>(request_id, msg.handle(peer_id, &req_environment))
                        .await
                        .ok();
                }
            })
            .await
    }
}

/// Handler that takes care of sending messages to a network, similar to a request except that we don't expect an answer.
pub fn message_handler<
    T: Send + Sync + Clone + 'static,
    Msg: MessageHandle<N, T> + Message,
    N: Network,
>(
    _network: &Arc<N>,
    stream: BoxStream<'static, (Msg, N::PeerId)>,
    req_environment: &T,
) -> impl Future<Output = ()> {
    let req_environment = req_environment.clone();
    async move {
        stream
            .for_each_concurrent(MAX_CONCURRENT_HANDLERS, |(msg, peer_id)| {
                let req_environment = req_environment.clone();
                async move {
                    // Messages do not have a response (so the response is ignored)
                    msg.message_handle(peer_id, &req_environment);
                }
            })
            .await
    }
}
