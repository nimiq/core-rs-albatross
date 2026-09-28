pub mod error;
pub mod network_impl;
pub mod single_response_requester;

use std::future::Future;

use futures::stream::BoxStream;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, SubscribeEvents, Topic},
    request::{Message, Request, RequestCommon},
    validator_record,
};
use nimiq_primitives::slots_allocation::Validators;

pub use crate::error::NetworkError;

pub type MessageStream<TMessage> = BoxStream<'static, (TMessage, u16)>;
pub type PubsubId<TValidatorNetwork> =
    <<TValidatorNetwork as ValidatorNetwork>::NetworkType as Network>::PubsubId;

/// Fixed upper bound network.
/// Peers are denoted by a usize identifier which deterministically identifies them.
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
    fn send_to<M: Message>(
        &self,
        validator_id: u16,
        msg: M,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Performs a request to a validator identified by its ID.
    fn request<TRequest: Request>(
        &self,
        request: TRequest,
        validator_id: u16,
    ) -> impl Future<
        Output = Result<
            <TRequest as RequestCommon>::Response,
            NetworkError<<Self::NetworkType as Network>::Error>,
        >,
    > + Send;

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
    fn respond<TRequest: Request>(
        &self,
        request_id: <Self::NetworkType as Network>::RequestId,
        response: TRequest::Response,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Publishes an item into a Gossipsub topic.
    fn publish<TTopic: Topic + Sync>(
        &self,
        item: TTopic::Item,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Subscribes to a specific Gossipsub topic.
    fn subscribe<'a, TTopic: Topic + Sync>(
        &self,
    ) -> impl Future<Output = Result<BoxStream<'a, (TTopic::Item, PubsubId<Self>)>, Self::Error>> + Send;

    /// Subscribes to network events
    fn subscribe_events(&self) -> SubscribeEvents<<Self::NetworkType as Network>::PeerId>;

    /// Sets this node peer ID using its secret key and public key.
    fn set_public_key(
        &self,
        validator_address: &Address,
        signing_key_pair: &KeyPair,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Closes the connection to the peer with `peer_id` with the given `close_reason`.
    fn disconnect_peer(
        &self,
        peer_id: <Self::NetworkType as Network>::PeerId,
        close_reason: CloseReason,
    ) -> impl Future<Output = ()> + Send;

    /// Signals that a Gossipsub'd message with `id` was verified successfully and can be relayed.
    fn validate_message<TTopic>(&self, id: PubsubId<Self>, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync;

    /// Returns the network peer ID for the given `validator_id` if it is known.
    fn get_peer_id(&self, validator_id: u16) -> Option<<Self::NetworkType as Network>::PeerId>;
}
