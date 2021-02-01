use std::fmt::Debug;
use std::hash::Hash;
use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use thiserror::Error;

use beserial::{Deserialize, Serialize, SerializingError};

use crate::message::Message;

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
    type Id: Debug + Hash + Eq;
    type Error: std::error::Error;

    fn id(&self) -> Self::Id;

    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError>;

    async fn send_or_close<T: Message, F: FnOnce(&SendError) -> CloseReason + Send>(&self, msg: &T, f: F) -> Result<(), SendError> {
        if let Err(e) = self.send(msg).await {
            log::error!("Sending failed: {}", e);
            self.close(f(&e));
            Err(e)
        } else {
            Ok(())
        }
    }

    /// Should panic if there is already a non-closed sink registered for a message type.
    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>>;

    fn close(&self, ty: CloseReason);

    async fn request<R: RequestResponse>(&self, request: &R::Request) -> Result<R::Response, Self::Error>;

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>>;
}
