use beserial::{Deserialize, Serialize, SerializingError};
use thiserror::Error;

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
