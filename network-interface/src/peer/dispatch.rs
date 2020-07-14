//! For dispatching to work we must modify the existing channels a bit:
//! - For any object to hold multiple channels it needs to dispatch the messages to,
//!   all those channels need to have a common type.
//! - We choose to have all channels of the type dyn Sink<Vec<u8>> with error type
//!   `DispatchError`.
//! - Thus, upon receiving a Vec<u8> we deserialize the message accordingly,
//!   before sending it over the channel.
use beserial::SerializingError;
use futures::channel::mpsc::{channel, unbounded, SendError};
use futures::sink::{Sink, SinkExt};
use futures::stream::{Stream, StreamExt};
use std::pin::Pin;

use crate::message::Message;

pub enum DispatchError {
    SendError(SendError),
    SerializingError(SerializingError),
}

impl From<SendError> for DispatchError {
    fn from(e: SendError) -> Self {
        DispatchError::SendError(e)
    }
}

impl From<SerializingError> for DispatchError {
    fn from(e: SerializingError) -> Self {
        DispatchError::SerializingError(e)
    }
}

pub fn bounded_dispatch<M: Message>(
    buffer: usize,
) -> (
    Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>,
    Pin<Box<dyn Stream<Item = M> + Send>>,
) {
    let (tx, rx) = channel(buffer);

    (
        Box::pin(tx.with(|msg: Vec<u8>| async move {
            M::deserialize_message(&mut &msg[..]).map_err(DispatchError::SerializingError)
        })),
        rx.boxed(),
    )
}

pub fn unbounded_dispatch<M: Message>() -> (
    Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>,
    Pin<Box<dyn Stream<Item = M> + Send>>,
) {
    let (tx, rx) = unbounded();

    (
        Box::pin(tx.with(|msg: Vec<u8>| async move {
            M::deserialize_message(&mut &msg[..]).map_err(DispatchError::SerializingError)
        })),
        rx.boxed(),
    )
}
