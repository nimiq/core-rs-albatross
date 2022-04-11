#[macro_use]
extern crate beserial_derive;

pub mod error;
pub mod network_impl;
pub mod validator_record;

use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{stream::BoxStream, Stream};

use nimiq_bls::{CompressedPublicKey, SecretKey};
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, PubsubId, Topic},
    request::Request,
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

    /// Tells the validator network the validator keys for the current set of active validators. The keys must be
    /// ordered, such that the k-th entry is the validator with ID k.
    async fn set_validators(&self, validator_keys: Vec<CompressedPublicKey>);

    /// must make a reasonable effort to establish a connection to the peer denoted with `validator_address`
    /// before returning a connection not established error.
    async fn send_to<Req: Request + Clone>(
        &self,
        validator_ids: &[usize],
        msg: Req,
    ) -> Vec<Result<(), Self::Error>>;

    /// Will receive from all connected peers
    fn receive<Req: Request + Clone>(
        &self,
    ) -> MessageStream<Req, <Self::NetworkType as Network>::PeerId>;

    async fn publish<TTopic: Topic + Sync>(&self, item: TTopic::Item) -> Result<(), Self::Error>;

    async fn subscribe<'a, TTopic: Topic + Sync>(
        &self,
    ) -> Result<BoxStream<'a, (TTopic::Item, Self::PubsubId)>, Self::Error>;

    /// registers a cache for the specified message type.
    /// Incoming messages of this type should be held in a FIFO queue of total size `buffer_size`, each with a lifetime of `lifetime`
    /// `lifetime` or `buffer_size` of 0 should disable the cache.
    fn cache<Req: Request>(&self, buffer_size: usize, lifetime: Duration);

    async fn set_public_key(
        &self,
        public_key: &CompressedPublicKey,
        secret_key: &SecretKey,
    ) -> Result<(), Self::Error>;

    /// Signals that a Gossipsup'd message with `id` was verified successfully and can be relayed
    fn validate_message<TTopic>(&self, id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync;
}
