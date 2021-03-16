use thiserror::Error;

use crate::behaviour::NimiqNetworkBehaviourError;


#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("Dial error: {0}")]
    Dial(#[from] libp2p::swarm::DialError),

    #[error("Failed to send action to swarm task: {0}")]
    Send(#[from] futures::channel::mpsc::SendError),

    #[error("Network action was cancelled: {0}")]
    Canceled(#[from] futures::channel::oneshot::Canceled),

    #[error("Serialization error: {0}")]
    Serialization(#[from] beserial::SerializingError),

    #[error("Network behaviour error: {0}")]
    Behaviour(#[from] NimiqNetworkBehaviourError),

    #[error("DHT store error: {0:?}")]
    DhtStore(libp2p::kad::store::Error),

    #[error("DHT GetRecord error: {0:?}")]
    DhtGetRecord(libp2p::kad::GetRecordError),

    #[error("DHT PutRecord error: {0:?}")]
    DhtPutRecord(libp2p::kad::PutRecordError),

    #[error("Gossipsub Publish error: {0:?}")]
    GossipsubPublish(libp2p::gossipsub::error::PublishError),

    #[error("Gossipsub Subscription error: {0:?}")]
    GossipsubSubscription(libp2p::gossipsub::error::SubscriptionError),

    #[error("Already subscribed to topic: {topic_name}")]
    AlreadySubscribed { topic_name: String },
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

impl From<libp2p::gossipsub::error::PublishError> for NetworkError {
    fn from(e: libp2p::gossipsub::error::PublishError) -> Self {
        Self::GossipsubPublish(e)
    }
}

impl From<libp2p::gossipsub::error::SubscriptionError> for NetworkError {
    fn from(e: libp2p::gossipsub::error::SubscriptionError) -> Self {
        Self::GossipsubSubscription(e)
    }
}
