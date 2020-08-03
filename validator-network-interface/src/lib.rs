use async_trait::async_trait;
use beserial::SerializingError;
use futures::Stream;
use std::pin::Pin;
use std::time::Duration;

use nimiq_network_interface::message::Message;

/// No notion of connected or disconnected!
/// If a peer is not connected the connection must be pursued.
/// If establishing the connection is impossible (i.e. Peer is offline or Self is offline) Unreachable is used.
#[derive(Debug)]
pub enum NetworkError {
    /// Serialization or deserialization of the message failed
    Serialization(SerializingError),
    /// Some of the peers were unreachable
    Unreachable,
    /// If no specific set of peers was given but no connection could be established indicating that self is unreachable
    Offline,
}

/// Fixed upper bound network.
/// Peers are denoted by a usize identifier which deterministically identifies them.
#[async_trait]
pub trait ValidatorNetwork: Send + Sync + 'static {
    /// must make a reasonable efford to establish a connection to the peer denoted with `validator_id`
    /// before returning a connection not established error.
    async fn send_to<M: Message>(
        &self,
        validator_ids: &[usize],
        msg: &M,
    ) -> Vec<Result<(), NetworkError>>;

    /// must make a reasonable efford to establish a connection to the peer denoted with `validator_id`
    /// before returning a connection not established error.
    fn receive_from<M: Message>(
        &self,
        validator_ids: &[usize],
    ) -> Pin<Box<dyn Stream<Item = (M, usize)> + Send>>;

    /// Will receive from all connected peers
    fn receive<M: Message>(&self) -> Pin<Box<dyn Stream<Item = (M, usize)>>>;

    /// In contrast to send_to will send to all available validators without having to try to establish a connection to
    /// any specific validatr very hard.
    async fn broadcast<M: Message>(&self, msg: &M) -> Result<(), NetworkError>;

    /// registers a cache for the specified message type.
    /// Incoming messages of this type shuld be held in a FIFO queue of total size `buffer_size`, each with a lifetime of `lifetime`
    /// `lifetime` or `buffer_size` of 0 should disable the cache.
    fn cache<M: Message>(&self, buffer_size: usize, lifetime: Duration);
}
