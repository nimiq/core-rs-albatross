use std::sync::Arc;

use futures::prelude::*;
use futures::stream::Forward;
use futures::sync::mpsc::*;

use crate::network::address::net_address::NetAddress;
use crate::network::connection::close_type::CloseType;
use crate::network::message::Message;
use crate::network::peer_channel::PeerSink;
use crate::network::peer_channel::PeerStream;
use crate::network::peer_channel::PeerChannel;
use crate::network::websocket::NimiqMessageStream;
use crate::network::websocket::SharedNimiqMessageStream;
use crate::network::address::peer_address::PeerAddress;
use parking_lot::RwLock;
use parking_lot::RwLockReadGuard;
use crate::network::websocket::NimiqMessageStreamError;
use crate::utils::observer::Notifier;
use crate::network::peer_channel::PeerStreamEvent;

#[derive(Clone)]
pub struct NetworkConnection<'conn> {
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    address_info: AddressInfo,
    pub notifier: Arc<RwLock<Notifier<'conn, PeerStreamEvent>>>,
}

impl<'conn> NetworkConnection<'conn> {
    pub fn new_connection_setup(stream: SharedNimiqMessageStream, address_info: AddressInfo) -> (Self, ProcessConnectionFuture<'conn>) {
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = rx.forward(stream.clone());

        let peer_sink = PeerSink::new(tx);
        let notifier = Arc::new(RwLock::new(Notifier::new()));
        let peer_stream = PeerStream::new(stream.clone(), notifier.clone());

        let network_connection = NetworkConnection {
            peer_sink,
            stream,
            address_info,
            notifier,
        };

        (network_connection, ProcessConnectionFuture::new(peer_stream, forward_future))
    }

    pub fn close(&self, ty: CloseType) -> CloseFuture {
        // Call `session.on_close()` eagerly, but only once.
        debug!("Closing connection, reason: {:?}", ty);
        self.notifier.read().notify(PeerStreamEvent::Close(ty));
        CloseFuture::new(self.stream.clone(), ty)
    }

    pub fn net_address(&self) -> Arc<NetAddress> {
        // We set net_address in the constructor.
        self.address_info.net_address().unwrap()
    }

    pub fn outbound(&self) -> bool {
        self.stream.outbound()
    }

    pub fn inbound(&self) -> bool {
        !self.outbound()
    }

    pub fn peer_address(&self) -> Option<Arc<PeerAddress>> { self.address_info.peer_address() }
    pub fn set_peer_address(&self, peer_address: Arc<PeerAddress>) {
        self.address_info.set_peer_address(peer_address);
    }

    pub fn peer_sink(&self) -> PeerSink { self.peer_sink.clone() }
    pub fn address_info(&self) -> AddressInfo {
        self.address_info.clone()
    }
}

pub struct ProcessConnectionFuture<'conn> {
    inner: Box<Future<Item=(), Error=()> + Send + Sync + 'conn>,
}

impl<'conn> ProcessConnectionFuture<'conn> {
    pub fn new(peer_stream: PeerStream<'conn>, forward_future: Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>) -> Self {
        let pair = forward_future.join(peer_stream.process_stream().map_err(|_| ())); // TODO: throwing away error info here
        ProcessConnectionFuture {
            inner: Box::new(pair.map(|_| ()))
        }
    }
}

impl<'conn> Future for ProcessConnectionFuture<'conn> {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Result<Async<<Self as Future>::Item>, <Self as Future>::Error> {
        self.inner.poll()
    }
}

pub struct CloseFuture {
    stream: SharedNimiqMessageStream,
    ty: CloseType,
}

impl CloseFuture {
    pub fn new(stream: SharedNimiqMessageStream, ty: CloseType) -> Self {
        CloseFuture {
            stream,
            ty
        }
    }
}

impl Future for CloseFuture {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        self.stream.close()
    }
}

pub struct AddressInfo {
    inner: Arc<AddressInfoInternal>,
}

struct AddressInfoInternal {
    pub peer_address: RwLock<Option<Arc<PeerAddress>>>,
    pub net_address: RwLock<Option<Arc<NetAddress>>>,
}

impl Clone for AddressInfo {
    fn clone(&self) -> Self {
        AddressInfo {
            inner: self.inner.clone()
        }
    }
}

impl AddressInfo {
    pub fn new(net_address: Option<Arc<NetAddress>>, peer_address: Option<Arc<PeerAddress>>) -> Self {
        AddressInfo {
            inner: Arc::new(AddressInfoInternal {
                peer_address: RwLock::new(peer_address),
                net_address: RwLock::new(net_address),
            })
        }
    }

    pub fn peer_address(&self) -> Option<Arc<PeerAddress>> {
        self.inner.peer_address.read().clone()
    }
    pub fn set_peer_address(&self, peer_address: Arc<PeerAddress>) {
        self.inner.peer_address.write().replace(peer_address);
    }
    pub fn net_address(&self) -> Option<Arc<NetAddress>> {
        self.inner.net_address.read().clone()
    }
    pub fn set_net_address(&self, net_address: Arc<NetAddress>) {
        self.inner.net_address.write().replace(net_address);
    }
}
