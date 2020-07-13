use async_trait::async_trait;
use beserial::SerializingError;
use futures::Stream;
use std::pin::Pin;

use crate::message::Message;

pub enum CloseReason {
    Other,
}

pub enum SendError {
    Serialization(SerializingError),
    AlreadyClosed,
}

#[async_trait]
pub trait Peer: Send + Sync {
    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError>;
    async fn send_or_close<T: Message, F: FnOnce(&SendError) -> CloseReason + Send>(
        &self,
        msg: &T,
        f: F,
    ) -> Result<(), SendError> {
        if let Err(e) = self.send(msg).await {
            self.close(f(&e)).await;
            Err(e)
        } else {
            Ok(())
        }
    }
    fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>>;
    async fn close(&self, ty: CloseReason);
}
