pub mod error;
pub mod network_impl;
pub mod single_response_requester;
pub mod validator_record;

use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{stream::BoxStream, Stream};
use nimiq_bls::{lazy::LazyPublicKey, CompressedPublicKey, SecretKey};
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, PubsubId, SubscribeEvents, Topic},
    request::{Message, Request, RequestCommon},
};

pub use crate::error::NetworkError;

pub type MessageStream<TMessage, TPeerId> =
    Pin<Box<dyn Stream<Item = (TMessage, TPeerId)> + Send + 'static>>;

/// Fixed upper bound network.
/// Peers are denoted by a usize identifier which deterministically identifies them.
#[async_trait]
pub trait ValidatorNetwork: Send + Sync {
    type Error: std::error::Error + Send + 'static;
    type NetworkType: Network;
    type PubsubId: PubsubId<<Self::NetworkType as Network>::PeerId> + Send;

    /// Tells the validator network the validator keys for the current set of active validators.
    /// The keys must be ordered, such that the k-th entry is the validator with ID k.
    async fn set_validators(&self, validator_keys: Vec<LazyPublicKey>);

    /// Sends a message to a validator identified by its ID (position) in the `validator keys`.
    /// It must make a reasonable effort to establish a connection to the peer denoted with `validator_id`
    /// before returning a connection not established error.
    async fn send_to<M: Message + Clone>(
        &self,
        validator_id: usize,
        msg: M,
    ) -> Result<(), Self::Error>;

    /// Performs a request to a validator identified by its ID.
    async fn request<TRequest: Request>(
        &self,
        request: TRequest,
        validator_id: usize,
    ) -> Result<
        <TRequest as RequestCommon>::Response,
        NetworkError<<Self::NetworkType as Network>::Error>,
    >;

    /// Returns a stream to receive certain types of messages from every peer.
    fn receive<M>(&self) -> MessageStream<M, <Self::NetworkType as Network>::PeerId>
    where
        M: Message + Clone;

    /// Publishes an item into a Gossipsub topic.
    async fn publish<TTopic: Topic + Sync>(&self, item: TTopic::Item) -> Result<(), Self::Error>;

    /// Subscribes to a specific Gossipsub topic.
    async fn subscribe<'a, TTopic: Topic + Sync>(
        &self,
    ) -> Result<BoxStream<'a, (TTopic::Item, Self::PubsubId)>, Self::Error>;

    /// Subscribes to network events
    fn subscribe_events(&self) -> SubscribeEvents<<Self::NetworkType as Network>::PeerId>;

    /// Registers a cache for the specified message type.
    /// Incoming messages of this type should be held in a FIFO queue of total size `buffer_size`,
    /// each with a lifetime of `lifetime`.
    /// `lifetime` or `buffer_size` of 0 should disable the cache.
    fn cache<M: Message>(&self, buffer_size: usize, lifetime: Duration);

    /// Sets this node peer ID using its secret key and public key.
    async fn set_public_key(
        &self,
        public_key: &CompressedPublicKey,
        secret_key: &SecretKey,
    ) -> Result<(), Self::Error>;

    /// Signals that a Gossipsup'd message with `id` was verified successfully and can be relayed.
    fn validate_message<TTopic>(&self, id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync;
}
