pub mod error;
pub mod network_impl;
pub mod single_response_requester;
pub mod validator_record;

use async_trait::async_trait;
use futures::stream::BoxStream;
use nimiq_bls::{lazy::LazyPublicKey, CompressedPublicKey, SecretKey};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, SubscribeEvents, Topic},
    request::{Message, Request, RequestCommon},
};

pub use crate::error::NetworkError;

pub type MessageStream<TMessage> = BoxStream<'static, (TMessage, usize)>;
pub type PubsubId<TValidatorNetwork> =
    <<TValidatorNetwork as ValidatorNetwork>::NetworkType as Network>::PubsubId;

/// Fixed upper bound network.
/// Peers are denoted by a usize identifier which deterministically identifies them.
#[async_trait]
pub trait ValidatorNetwork: Send + Sync {
    type Error: std::error::Error + Send + 'static;
    type NetworkType: Network;

    /// Tells the validator network its own validator ID in case it is an active validator, or
    /// `None`, otherwise.
    fn set_validator_id(&self, validator_id: Option<u16>);

    /// Tells the validator network the validator keys for the current set of active validators.
    /// The keys must be ordered, such that the k-th entry is the validator with ID k.
    async fn set_validators(&self, validator_keys: Vec<LazyPublicKey>);

    /// Sends a message to a validator identified by its ID (position) in the `validator keys`.
    /// It must make a reasonable effort to establish a connection to the peer denoted with `validator_id`
    /// before returning a connection not established error.
    async fn send_to<M: Message>(&self, validator_id: u16, msg: M) -> Result<(), Self::Error>;

    /// Performs a request to a validator identified by its ID.
    async fn request<TRequest: Request>(
        &self,
        request: TRequest,
        validator_id: u16,
    ) -> Result<
        <TRequest as RequestCommon>::Response,
        NetworkError<<Self::NetworkType as Network>::Error>,
    >;

    /// Returns a stream to receive certain types of messages from every peer.
    fn receive<M>(&self) -> MessageStream<M>
    where
        M: Message + Clone;

    /// Publishes an item into a Gossipsub topic.
    async fn publish<TTopic: Topic + Sync>(&self, item: TTopic::Item) -> Result<(), Self::Error>;

    /// Subscribes to a specific Gossipsub topic.
    async fn subscribe<'a, TTopic: Topic + Sync>(
        &self,
    ) -> Result<BoxStream<'a, (TTopic::Item, PubsubId<Self>)>, Self::Error>;

    /// Subscribes to network events
    fn subscribe_events(&self) -> SubscribeEvents<<Self::NetworkType as Network>::PeerId>;

    /// Sets this node peer ID using its secret key and public key.
    async fn set_public_key(
        &self,
        public_key: &CompressedPublicKey,
        secret_key: &SecretKey,
    ) -> Result<(), Self::Error>;

    /// Closes the connection to the peer with `peer_id` with the given `close_reason`.
    async fn disconnect_peer(
        &self,
        peer_id: <Self::NetworkType as Network>::PeerId,
        close_reason: CloseReason,
    );

    /// Signals that a Gossipsub'd message with `id` was verified successfully and can be relayed.
    fn validate_message<TTopic>(&self, id: PubsubId<Self>, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync;
}
