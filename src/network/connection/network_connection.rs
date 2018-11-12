use std::sync::Arc;

use futures::prelude::*;
use futures::stream::Forward;
use futures::sync::mpsc::*;

use crate::network::address::net_address::NetAddress;
use crate::network::connection::close_type::CloseType;
use crate::network::message::Message;
use crate::network::peer_channel::PeerSink;
use crate::network::peer_channel::PeerStream;
use crate::network::peer_channel::Session;
use crate::network::websocket::NimiqMessageStream;
use crate::network::websocket::SharedNimiqMessageStream;
use crate::network::address::peer_address::PeerAddress;
use parking_lot::RwLock;
use parking_lot::RwLockReadGuard;

pub struct NetworkConnection {
    peer_stream: Option<PeerStream>,
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    forward_future: Option<Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>>,
    session: Arc<Session>,
    address_info: AddressInfo,
}

impl NetworkConnection {
    pub fn new(stream: NimiqMessageStream, address_info: Option<AddressInfo>) -> Self {
        let shared_stream: SharedNimiqMessageStream = stream.into();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = Some(rx.forward(shared_stream.clone()));

        let net_address = Arc::new(shared_stream.net_address().clone());
        let address_info = address_info.unwrap_or(AddressInfo::new(None, None));
        address_info.set_net_address(net_address);

        let peer_sink = PeerSink::new(tx, address_info.clone());
        let session = Arc::new(Session::new(peer_sink.clone()));

        NetworkConnection {
            peer_stream: Some(PeerStream::new(shared_stream.clone(), session.clone())),
            peer_sink,
            stream: shared_stream,
            forward_future,
            session,
            address_info,
        }
    }

    pub fn process_connection(&mut self) -> impl Future<Item=(), Error=()> {
        assert!(self.forward_future.is_some() && self.peer_stream.is_some(), "Process connection can only be called once!");

        self.session.initialize();

        let forward_future = self.forward_future.take().unwrap();
        let stream = self.peer_stream.take().unwrap();
        let pair = forward_future.join(stream.process_stream().map_err(|_| ())); // TODO: throwing away error info here
        pair.map(|_| ())
    }

    pub fn close(&self, ty: CloseType) -> CloseFuture {
        CloseFuture::new(self.session.clone(), self.stream.clone(), ty)
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

    pub fn address_info(&self) -> AddressInfo {
        self.address_info.clone()
    }
}

pub struct CloseFuture {
    session: Option<Arc<Session>>,
    stream: SharedNimiqMessageStream,
    ty: CloseType,
}

impl CloseFuture {
    pub fn new(session: Arc<Session>, stream: SharedNimiqMessageStream, ty: CloseType) -> Self {
        CloseFuture {
            session: Some(session),
            stream,
            ty
        }
    }
}

impl Future for CloseFuture {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        // Call `session.on_close()` eagerly, but only once.
        if let Some(session) = self.session.take() {
            debug!("Closing connection, reason: {:?}", self.ty);
            session.on_close();
        }
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
