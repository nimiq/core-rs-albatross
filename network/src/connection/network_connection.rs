use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::prelude::*;
use futures::stream::Forward;
use futures::sync::mpsc::*;
use parking_lot::Mutex;
use parking_lot::RwLock;

use network_primitives::address::net_address::NetAddress;
use network_primitives::address::peer_address::PeerAddress;
use utils::observer::PassThroughNotifier;
use utils::unique_id::UniqueId;

use crate::connection::close_type::CloseType;
#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::peer_channel::PeerSink;
use crate::peer_channel::PeerStream;
use crate::peer_channel::PeerStreamEvent;
use crate::websocket::{Message, SharedNimiqMessageStream};
use std::fmt;

#[derive(Debug, Clone, Default)]
pub struct ClosedFlag {
    closed: Arc<AtomicBool>,
    close_type: Arc<Mutex<Option<CloseType>>>,
}

impl ClosedFlag {
    #[inline]
    pub fn new() -> Self {
        ClosedFlag {
            closed: Arc::new(AtomicBool::new(false)),
            close_type: Arc::new(Mutex::new(None)),
        }
    }

    #[inline]
    pub fn set_closed(&self, closed: bool) -> bool {
        self.closed.swap(closed, Ordering::AcqRel)
    }
    #[inline]
    pub fn set_close_type(&self, ty: CloseType) {
        self.close_type.lock().replace(ty);
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    #[inline]
    pub fn close_type(&self) -> Option<CloseType> {
        *self.close_type.lock()
    }
}

#[derive(Clone)]
pub struct NetworkConnection {
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    address_info: AddressInfo,
    unique_id: UniqueId,
    closed_flag: ClosedFlag,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
}

impl NetworkConnection {
    pub fn new_connection_setup(stream: SharedNimiqMessageStream, address_info: AddressInfo) -> (Self, ProcessConnectionFuture) {
        let id = UniqueId::new();
        let closed_flag = ClosedFlag::new();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = rx.forward(stream.clone());

        let notifier = Arc::new(RwLock::new(PassThroughNotifier::new()));
        let peer_stream = PeerStream::new(stream.clone(), notifier.clone(), closed_flag.clone());
        let process_connection = ProcessConnectionFuture::new(peer_stream, forward_future, id);

        let peer_sink = PeerSink::new(tx, id, closed_flag.clone());

        let network_connection = NetworkConnection {
            peer_sink,
            stream,
            address_info,
            notifier,
            closed_flag,
            unique_id: id,
        };

        (network_connection, process_connection)
    }

    pub fn close(&self, ty: CloseType) {
        if let Err(error) = self.peer_sink.close(ty, None) {
            debug!("Error closing connection to {}: {}",
                self.address_info, error);
        }
        let notifier = Arc::clone(&self.notifier);
        tokio::spawn(futures::lazy(move || {
            notifier.read().notify(PeerStreamEvent::Close(ty));
            futures::future::ok(())
        }));
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

    pub fn closed(&self) -> bool {
        self.closed_flag.is_closed()
    }
    pub fn closed_flag(&self) -> ClosedFlag {
        self.closed_flag.clone()
    }

    pub fn peer_address(&self) -> Option<Arc<PeerAddress>> { self.address_info.peer_address() }
    pub fn set_peer_address(&self, peer_address: Arc<PeerAddress>) {
        self.address_info.set_peer_address(peer_address);
    }

    pub fn peer_sink(&self) -> PeerSink { self.peer_sink.clone() }
    pub fn address_info(&self) -> AddressInfo {
        self.address_info.clone()
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &Arc<NetworkMetrics> {
        self.stream.network_metrics()
    }
}

pub struct ProcessConnectionFuture {
    inner: Box<dyn Future<Item=(), Error=()> + Send + Sync + 'static>,
}

impl ProcessConnectionFuture {
    pub fn new(peer_stream: PeerStream, forward_future: Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>, _id: UniqueId) -> Self {
        // `select` required Item/Error to be the same, that's why we need to map them both to ().
        // TODO We're discarding any errors here, especially those coming from the forward future.
        // Results by the peer_stream have been processes already.
        let connection = forward_future.map(|_| ()).map_err(|_| ()).select(peer_stream.process_stream().map_err(|_| ()));
        let connection = connection.map(|_| ()).map_err(|_| ());

        Self {
            inner: Box::new(connection)
        }
    }
}

impl Future for ProcessConnectionFuture {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Result<Async<<Self as Future>::Item>, <Self as Future>::Error> {
        self.inner.poll()
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

impl fmt::Display for AddressInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "AddressInfo {{")?;
        match self.inner.peer_address.try_read() {
            Some(lock) => write!(f, "peer_address: {}, ", match lock.as_ref() {
                Some(address) => address.to_string(),
                None => "None".to_string(),
            })?,
            None => write!(f, "peer_address: [locked], ")?,
        }
        match self.inner.net_address.try_read() {
            Some(lock) => write!(f, "net_address: {}, ", match lock.as_ref() {
                Some(address) => address.to_string(),
                None => "None".to_string(),
            })?,
            None => write!(f, "net_address: [locked], ")?,
        }
        write!(f, "}}")
    }
}
