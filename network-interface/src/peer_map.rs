use std::{
    collections::HashMap,
    sync::Arc,
    iter::FromIterator,
};
use tokio::sync::broadcast;
use parking_lot::RwLock;

use crate::{
    network::NetworkEvent,
    peer::Peer,
};


struct Inner<P>
    where P: Peer + 'static,
{
    peers: HashMap<P::Id, Arc<P>>,
    tx: broadcast::Sender<NetworkEvent<P>>,
}

impl<P> Inner<P>
    where P: Peer + 'static,
{
    fn notify(&self, event: NetworkEvent<P>) {
        // According to documentation this only fails if all receivers dropped. But that's okay for us.
        self.tx.send(event).ok();
    }
}

pub struct ObservablePeerMap<P>
    where P: Peer + 'static,
{
    inner: Arc<RwLock<Inner<P>>>,
}

impl<P> Clone for ObservablePeerMap<P>
    where P: Peer + std::fmt::Debug,
{
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<P> std::fmt::Debug for ObservablePeerMap<P>
    where P: Peer + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        self.inner.read().peers.fmt(f)
    }
}

impl<P> Default for ObservablePeerMap<P>
    where P: Peer,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<P> FromIterator<Arc<P>> for ObservablePeerMap<P>
    where P: Peer,
{
    fn from_iter<I: IntoIterator<Item=Arc<P>>>(iter: I) -> Self {
        Self::from_peers(iter.into_iter().map(|peer| (peer.id(), peer)).collect())
    }
}

impl<P> FromIterator<P> for ObservablePeerMap<P>
    where P: Peer,
{
    fn from_iter<I: IntoIterator<Item=P>>(iter: I) -> Self {
        Self::from_iter(iter.into_iter().map(|peer| Arc::new(peer)))
    }
}


impl<P> ObservablePeerMap<P>
    where P: Peer + 'static,
{
    fn from_peers(peers: HashMap<P::Id, Arc<P>>) -> Self {
        let (tx, _rx) = broadcast::channel(64);

        Self {
            inner: Arc::new(RwLock::new(Inner {
                peers,
                tx,
            }))
        }
    }

    pub fn new() -> Self {
        Self::from_peers(HashMap::new())
    }

    pub fn insert(&self, peer: impl Into<Arc<P>>) {
        let peer = peer.into();

        let mut inner = self.inner.write();

        if inner.peers.insert(peer.id(), Arc::clone(&peer)).is_none() {
            inner.notify(NetworkEvent::PeerJoined(peer));
        }
    }

    pub fn remove(&self, peer_id: &P::Id) -> Option<Arc<P>> {
        let mut inner = self.inner.write();

        if let Some(peer) = inner.peers.remove(peer_id) {
            inner.notify(NetworkEvent::PeerLeft(Arc::clone(&peer)));
            Some(peer)
        }
        else {
            None
        }
    }

    pub fn get_peer(&self, peer_id: &P::Id) -> Option<Arc<P>> {
        self.inner
            .read()
            .peers
            .get(peer_id)
            .map(|peer| Arc::clone(peer))
    }

    pub fn get_peers(&self) -> Vec<Arc<P>> {
        self.inner
            .read()
            .peers
            .values()
            .map(|peer| Arc::clone(peer))
            .collect()
    }

    pub fn subscribe(&self) -> (Vec<Arc<P>>, broadcast::Receiver<NetworkEvent<P>>) {
        let inner = self.inner.write();

        let peers = inner.peers
            .values()
            .map(|peer| Arc::clone(peer))
            .collect();

        let rx = inner.tx.subscribe();

        (peers, rx)
    }
}



#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        pin::Pin,
    };

    use futures::{Stream, StreamExt};
    use thiserror::Error;
    use tokio::sync::broadcast;

    use crate::{
        network::NetworkEvent,
        peer::{Peer as PeerInterface, SendError, CloseReason, RequestResponse},
        message::Message,
    };
    use super::ObservablePeerMap;

    #[derive(Debug, Error)]
    pub enum PeerError {}

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct Peer {
        pub id: u32,
    }

    impl Peer {
        pub fn new(id: u32) -> Self {
            Self { id }
        }
    }

    #[async_trait::async_trait]
    impl PeerInterface for Peer {
        type Id = u32;
        type Error = PeerError;

        fn id(&self) -> Self::Id {
            self.id
        }

        async fn send<T: Message>(&self, _msg: &T) -> Result<(), SendError> { unreachable!(); }
        fn receive<T: Message>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> { unreachable!(); }
        fn close(&self, _ty: CloseReason) {}

        async fn request<R: RequestResponse>(&self, _request: &R::Request) -> Result<R::Response, Self::Error> { unreachable!(); }

        fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> { unreachable!(); }
    }

    async fn assert_peer_joined(listener: &mut broadcast::Receiver<NetworkEvent<Peer>>, id: u32) {
        if let Some(Ok(NetworkEvent::PeerJoined(peer))) = listener.next().await {
            assert_eq!(peer.id(), id);
        }
        else {
            panic!("Expected PeerJoined event with id={}", id);
        }
    }

    async fn assert_peer_left(listener: &mut broadcast::Receiver<NetworkEvent<Peer>>, id: u32) {
        if let Some(Ok(NetworkEvent::PeerLeft(peer))) = listener.next().await {
            assert_eq!(peer.id(), id);
        }
        else {
            panic!("Expected PeerLeft event with id={}", id);
        }
    }

    #[tokio::test]
    async fn it_adds_peers() {
        let peers = ObservablePeerMap::new();

        peers.insert(Peer::new(1));
        peers.insert(Peer::new(2));

        let (current_peers, mut listener) = peers.subscribe();

        peers.insert(Peer::new(3));
        peers.insert(Peer::new(2)); // This must not emit an event
        peers.insert(Peer::new(4));

        let current_peer_ids: HashSet<u32> = current_peers.iter().map(|p| p.id()).collect();
        assert_eq!(current_peer_ids, [1, 2].iter().copied().collect());

        assert_peer_joined(&mut listener, 3).await;
        assert_peer_joined(&mut listener, 4).await;
    }

    #[tokio::test]
    async fn it_removes_peers() {
        let peers = ObservablePeerMap::new();

        peers.insert(Peer::new(1));
        peers.insert(Peer::new(2));
        peers.insert(Peer::new(3));

        peers.remove(&2);

        let (current_peers, mut listener) = peers.subscribe();

        peers.remove(&1);

        let current_peer_ids: HashSet<u32> = current_peers.iter().map(|p| p.id()).collect();
        assert_eq!(current_peer_ids, [1, 3].iter().copied().collect());

        assert_peer_left(&mut listener, 1).await;
    }
}
