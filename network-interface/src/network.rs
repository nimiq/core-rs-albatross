use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use futures::{future::BoxFuture, stream::BoxStream};
use tokio_stream::wrappers::BroadcastStream;

use beserial::{Deserialize, Serialize};

use crate::{
    message::{Message, RequestError, ResponseMessage},
    peer::*,
};

pub enum NetworkEvent<P> {
    PeerJoined(Arc<P>),
    PeerLeft(Arc<P>),
}

pub trait Topic {
    type Item: Serialize + Deserialize + Send + Sync + std::fmt::Debug + 'static;

    const BUFFER_SIZE: usize;
    const NAME: &'static str;
    const VALIDATE: bool;
}

impl<P: Peer> std::fmt::Debug for NetworkEvent<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let (event_name, peer) = match self {
            NetworkEvent::PeerJoined(peer) => ("PeerJoined", peer),
            NetworkEvent::PeerLeft(peer) => ("PeerLeft", peer),
        };

        f.debug_struct(event_name)
            .field("peer_id", &peer.id())
            .finish()
    }
}

impl<P> Clone for NetworkEvent<P> {
    fn clone(&self) -> Self {
        match self {
            NetworkEvent::PeerJoined(peer) => NetworkEvent::PeerJoined(Arc::clone(peer)),
            NetworkEvent::PeerLeft(peer) => NetworkEvent::PeerLeft(Arc::clone(peer)),
        }
    }
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
pub trait Network: Send + Sync + 'static {
    type PeerType: Peer + 'static;
    type AddressType: std::fmt::Display + std::fmt::Debug;
    type Error: std::error::Error;
    type PubsubId: PubsubId<<Self::PeerType as Peer>::Id>;
    type RequestId: Debug + Copy + Clone + PartialEq + Eq + Send + Sync;

    fn get_peer_updates(
        &self,
    ) -> (
        Vec<Arc<Self::PeerType>>,
        BroadcastStream<NetworkEvent<Self::PeerType>>,
    );

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>>;
    fn get_peer(&self, peer_id: <Self::PeerType as Peer>::Id) -> Option<Arc<Self::PeerType>>;

    fn subscribe_events(&self) -> BroadcastStream<NetworkEvent<Self::PeerType>>;

    async fn subscribe<'a, T>(
        &self,
    ) -> Result<BoxStream<'a, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync;

    async fn unsubscribe<'a, T>(&self) -> Result<(), Self::Error>
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

    async fn dial_peer(&self, peer_id: <Self::PeerType as Peer>::Id) -> Result<(), Self::Error>;

    async fn dial_address(&self, address: Self::AddressType) -> Result<(), Self::Error>;

    fn get_local_peer_id(&self) -> <Self::PeerType as Peer>::Id;

    async fn request<'a, Req: Message, Res: Message>(
        &self,
        request: Req,
        peer_id: <Self::PeerType as Peer>::Id,
    ) -> Result<
        BoxFuture<
            'a,
            (
                ResponseMessage<Res>,
                Self::RequestId,
                <Self::PeerType as Peer>::Id,
            ),
        >,
        RequestError,
    >;

    fn receive_requests<'a, M: Message>(
        &self,
    ) -> BoxStream<'a, (M, Self::RequestId, <Self::PeerType as Peer>::Id)>;

    async fn respond<'a, M: Message>(
        &self,
        request_id: Self::RequestId,
        response: M,
    ) -> Result<(), Self::Error>;
}
