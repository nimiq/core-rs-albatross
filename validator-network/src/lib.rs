pub mod error;
pub mod network_impl;
pub mod single_response_requester;

use async_trait::async_trait;
use futures::stream::BoxStream;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, SubscribeEvents, Topic},
    request::{Message, Request, RequestCommon},
    validator_record::ValidatorRecord,
};
use nimiq_primitives::slots_allocation::Validators;
use nimiq_utils::tagged_signing::TaggedSigned;

pub use crate::error::NetworkError;

pub type MessageStream<TMessage> = BoxStream<'static, (TMessage, u16)>;
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

    /// Tells the validator network the validator addresses for the current set of active validators.
    /// The keys must be ordered, such that the k-th entry is the validator with ID k.
    fn set_validators(&self, validators: &Validators);

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

    /// Receives requests from peers.
    /// This function returns a stream where the requests are going to be propagated.
    fn receive_requests<TRequest: Request>(
        &self,
    ) -> BoxStream<'static, (TRequest, <Self::NetworkType as Network>::RequestId, u16)>;

    /// Sends a response to a specific request.
    async fn respond<TRequest: Request>(
        &self,
        request_id: <Self::NetworkType as Network>::RequestId,
        response: TRequest::Response,
    ) -> Result<(), Self::Error>;

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
        validator_address: &Address,
        signing_key_pair: &KeyPair,
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

    /// Returns the network peer ID for the given `validator_id` if it is known.
    fn get_peer_id(&self, validator_id: u16) -> Option<<Self::NetworkType as Network>::PeerId>;

    /// Registers a callback to produce a signed ValidatorRecord from a given peer_id and timestamp.
    fn register_validator_signing_callback(
        &self,
        callback: impl Fn(
                <Self::NetworkType as Network>::PeerId,
                u64,
            )
                -> TaggedSigned<ValidatorRecord<<Self::NetworkType as Network>::PeerId>, KeyPair>
            + Send
            + Sync
            + 'static,
    );
}
