use std::fmt::Debug;
use std::hash::Hash;

use async_trait::async_trait;
use thiserror::Error;

use beserial::{Deserialize, Serialize, SerializingError};

pub mod dispatch;

#[derive(Copy, Clone, Debug)]
pub enum CloseReason {
    Other,
    RemoteClosed,
    Error,
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("{0}")]
    Serialization(#[from] SerializingError),
    #[error("Peer connection already closed")]
    AlreadyClosed,
}

pub trait RequestResponse {
    type Request: Serialize + Deserialize + Sync;
    type Response: Serialize + Deserialize + Sync;
}

#[async_trait]
// TODO: Use Peer::Error for returned error for send, etc.
pub trait Peer: Send + Sync + Hash + Eq {
    type Id: Clone + Copy + Debug + Send + Sync + Hash + Eq + Unpin;
    type Error: std::error::Error;

    fn id(&self) -> Self::Id;

    fn close(&self, ty: CloseReason);
}
