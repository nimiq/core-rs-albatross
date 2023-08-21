use thiserror::Error;

use crate::dispatch::codecs::typed::MessageCodec;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("Dial error: {0}")]
    Dial(#[from] libp2p::swarm::DialError),

    #[error("Failed to send action to swarm task")]
    Send,

    #[error("Network action was cancelled")]
    Cancelled,

    #[error("We could not find any peer that satisfies the desired services")]
    PeersNotFound,

    #[error("Serialization error: {0}")]
    Serialization(#[from] nimiq_serde::DeserializeError),

    #[error("DHT store error: {0:?}")]
    DhtStore(libp2p::kad::store::Error),

    #[error("DHT GetRecord error: {0:?}")]
    DhtGetRecord(libp2p::kad::GetRecordError),

    #[error("DHT PutRecord error: {0:?}")]
    DhtPutRecord(libp2p::kad::PutRecordError),

    #[error("Gossipsub Publish error: {0:?}")]
    GossipsubPublish(libp2p::gossipsub::PublishError),

    #[error("Gossipsub Subscription error: {0:?}")]
    GossipsubSubscription(libp2p::gossipsub::SubscriptionError),

    #[error("Already subscribed to topic: {topic_name}")]
    AlreadySubscribed { topic_name: String },

    #[error("Already unsubscribed to topic: {topic_name}")]
    AlreadyUnsubscribed { topic_name: String },

    #[error("Unknown Request ID")]
    UnknownRequestId,

    #[error("Couldn't set topic score parameters")]
    TopicScoreParams {
        topic_name: String,
        error: &'static str,
    },
    #[error("Response channel closed: {0:?}")]
    ResponseChannelClosed(<MessageCodec as libp2p::request_response::Codec>::Response),
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for NetworkError {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        NetworkError::Send
    }
}

impl From<tokio::sync::oneshot::error::RecvError> for NetworkError {
    fn from(_: tokio::sync::oneshot::error::RecvError) -> Self {
        NetworkError::Cancelled
    }
}

impl From<libp2p::kad::store::Error> for NetworkError {
    fn from(e: libp2p::kad::store::Error) -> Self {
        Self::DhtStore(e)
    }
}

impl From<libp2p::kad::GetRecordError> for NetworkError {
    fn from(e: libp2p::kad::GetRecordError) -> Self {
        Self::DhtGetRecord(e)
    }
}

impl From<libp2p::kad::PutRecordError> for NetworkError {
    fn from(e: libp2p::kad::PutRecordError) -> Self {
        Self::DhtPutRecord(e)
    }
}

impl From<libp2p::gossipsub::PublishError> for NetworkError {
    fn from(e: libp2p::gossipsub::PublishError) -> Self {
        Self::GossipsubPublish(e)
    }
}

impl From<libp2p::gossipsub::SubscriptionError> for NetworkError {
    fn from(e: libp2p::gossipsub::SubscriptionError) -> Self {
        Self::GossipsubSubscription(e)
    }
}
