use std::fmt::{Debug, Display};
use std::hash::Hash;

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use beserial::{Deserialize, Serialize};

use crate::{
    peer::*,
    request::{Message, Request, RequestError},
};

#[derive(Clone, Debug)]
pub enum NetworkEvent<P> {
    PeerJoined(P),
    PeerLeft(P),
}

pub type SubscribeEvents<PeerId> =
    BoxStream<'static, Result<NetworkEvent<PeerId>, BroadcastStreamRecvError>>;

pub trait Topic {
    type Item: Serialize + Deserialize + Send + Sync + Debug + 'static;

    const BUFFER_SIZE: usize;
    const NAME: &'static str;
    const VALIDATE: bool;
}

// It seems we can't use type aliases on enums yet:
// https://rust-lang.github.io/rfcs/2338-type-alias-enum-variants.html
#[derive(Clone, Debug)]
pub enum MsgAcceptance {
    Accept,
    Reject,
    Ignore,
}

pub trait PubsubId<PeerId>: Clone + Send + Sync {
    fn propagation_source(&self) -> PeerId;
}

#[async_trait]
pub trait Network: Send + Sync + Unpin + 'static {
    type PeerId: Copy + Debug + Display + Eq + Hash + Send + Sync + Unpin + 'static;
    type AddressType: Debug + Display + 'static;
    type Error: std::error::Error;
    type PubsubId: PubsubId<Self::PeerId> + Send + Sync + Unpin;
    type RequestId: Copy + Debug + Display + Eq + Send + Sync + 'static;

    fn get_peers(&self) -> Vec<Self::PeerId>;
    fn has_peer(&self, peer_id: Self::PeerId) -> bool;
    fn peer_provides_required_services(&self, peer_id: Self::PeerId) -> bool;
    async fn disconnect_peer(&self, peer_id: Self::PeerId, close_reason: CloseReason);

    fn subscribe_events(&self) -> SubscribeEvents<Self::PeerId>;

    async fn subscribe<T>(
        &self,
    ) -> Result<BoxStream<'static, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync;

    async fn unsubscribe<T>(&self) -> Result<(), Self::Error>
    where
        T: Topic + Sync;

    async fn publish<T>(&self, item: T::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync;

    fn validate_message<T>(&self, id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        T: Topic + Sync;

    async fn dht_get<K, V>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync;

    async fn dht_put<K, V>(&self, k: &K, v: &V) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync;

    async fn dial_peer(&self, peer_id: Self::PeerId) -> Result<(), Self::Error>;

    async fn dial_address(&self, address: Self::AddressType) -> Result<(), Self::Error>;

    fn get_local_peer_id(&self) -> Self::PeerId;

    async fn message<M: Message>(
        &self,
        request: M,
        peer_id: Self::PeerId,
    ) -> Result<(), RequestError>;

    async fn request<Req: Request>(
        &self,
        request: Req,
        peer_id: Self::PeerId,
    ) -> Result<Req::Response, RequestError>;

    fn receive_messages<M: Message>(&self) -> BoxStream<'static, (M, Self::PeerId)>;

    fn receive_requests<Req: Request>(
        &self,
    ) -> BoxStream<'static, (Req, Self::RequestId, Self::PeerId)>;

    async fn respond<Req: Request>(
        &self,
        request_id: Self::RequestId,
        response: Req::Response,
    ) -> Result<(), Self::Error>;
}
