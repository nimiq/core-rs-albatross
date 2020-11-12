use std::fmt::Debug;
use std::hash::Hash;
use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use thiserror::Error;

use beserial::SerializingError;

use crate::message::Message;

pub mod dispatch;

#[derive(Debug)]
pub enum CloseReason {
    Other,
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("{0}")]
    Serialization(#[from] SerializingError),
    #[error("Peer connection already closed")]
    AlreadyClosed,
}

#[async_trait]
pub trait Peer: Send + Sync + Hash + Eq {
    type Id: Debug;

    fn id(&self) -> Self::Id;
    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError>;
    async fn send_or_close<T: Message, F: FnOnce(&SendError) -> CloseReason + Send>(&self, msg: &T, f: F) -> Result<(), SendError> {
        if let Err(e) = self.send(msg).await {
            self.close(f(&e)).await;
            Err(e)
        } else {
            Ok(())
        }
    }
    /// Should panic if there is already a non-closed sink registered for a message type.
    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>>;
    async fn close(&self, ty: CloseReason);
}
