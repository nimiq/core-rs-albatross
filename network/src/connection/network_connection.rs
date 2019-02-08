use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::prelude::*;
use futures::stream::Forward;
use futures::sync::mpsc::*;
use futures::sync::oneshot;
use parking_lot::Mutex;
use parking_lot::RwLock;

use network_messages::Message;
use network_primitives::address::net_address::NetAddress;
use network_primitives::address::peer_address::PeerAddress;
use utils::observer::PassThroughNotifier;

use crate::connection::close_type::CloseType;
#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::peer_channel::PeerSink;
use crate::peer_channel::PeerStream;
use crate::peer_channel::PeerStreamEvent;
use crate::websocket::SharedNimiqMessageStream;

#[derive(Clone)]
pub struct NetworkConnection {
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    address_info: AddressInfo,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
}

impl NetworkConnection {
    pub fn new_connection_setup(stream: SharedNimiqMessageStream, address_info: AddressInfo) -> (Self, ProcessConnectionFuture) {
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = rx.forward(stream.clone());

        let notifier = Arc::new(RwLock::new(PassThroughNotifier::new()));
        let peer_stream = PeerStream::new(stream.clone(), notifier.clone());
        let (process_connection, closing_helper) = ProcessConnectionFuture::new(peer_stream, forward_future, stream.clone());

        let peer_sink = PeerSink::new(tx, closing_helper);

        let network_connection = NetworkConnection {
            peer_sink,
            stream,
            address_info,
            notifier,
        };

        (network_connection, process_connection)
    }

    pub fn close(&self, ty: CloseType) -> bool {
        self.peer_sink.close(ty)
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

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &Arc<NetworkMetrics> {
        self.stream.network_metrics()
    }
}

pub struct ClosingHelper {
    closing_tx: Mutex<Option<oneshot::Sender<CloseType>>>,
    closed: AtomicBool,
    notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
}

impl ClosingHelper {
    pub fn new(closing_tx: oneshot::Sender<CloseType>,
               notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>) -> Self {
        Self {
            closing_tx: Mutex::new(Some(closing_tx)),
            notifier,
            closed: AtomicBool::new(false),
        }
    }

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn close(&self, ty: CloseType) -> bool {
        debug!("Closing connection, reason: {:?}", ty);

        // Don't close if already done and atomically mark as closed.
        if self.closed.swap(true, Ordering::Release) {
            return false;
        }

        // Send out oneshot with CloseType to close the connection from our end.
        let mut closing_tx = self.closing_tx.lock();
        assert!(closing_tx.is_some(), "Trying to close already closed connection.");
        let closing_tx = closing_tx.take().unwrap();
        if closing_tx.send(ty).is_err() {
            // Already closed by remote.
            return false;
        }

        // Now send out notifications.
        self.notifier.read().notify(PeerStreamEvent::Close(ty));
        return true;
    }
}

pub struct ProcessConnectionFuture {
    inner: Box<Future<Item=(), Error=()> + Send + Sync + 'static>,
}

impl ProcessConnectionFuture {
    pub fn new(peer_stream: PeerStream, forward_future: Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>, mut shared_stream: SharedNimiqMessageStream) -> (Self, ClosingHelper) {
        let (closing_tx, closing_rx) = oneshot::channel::<CloseType>();
        let notifier = peer_stream.notifier.clone();

        // `select` required Item/Error to be the same, that's why we need to map them both to ().
        let connection = forward_future.map(|_| ()).map_err(|_| ()).select(peer_stream.process_stream().map_err(|_| ())); // TODO: throwing away error info here
        // `select` required Item/Error to be the same.
        // `closing_rx` has Item=ClosingType, which we want to use. But Error will be set to ().
        let closing_future = closing_rx.map_err(|_| ());
        // `connection` currently has a tuple type, but we will soon call `select`, so unify Error to ()
        // and set Item=CloseType with `CloseType::Regular` to identify a normal shutdown.
        let connection = connection.map(|_| CloseType::Regular).map_err(|_| ());
        // Types have already been unified, so select over normal connection and `closing_future`.
        let connection = connection.select(closing_future);
        // If the connection is to be dropped because of a forceful close, CloseType != `CloseType::Regular`.
        // So, in these cases, send the WS CloseFrame.
        // Set Item=() and map Error=().
        let connection = connection.and_then(move |(ty, _)| {
            // Specifically send close frame if CloseType is not Regular.
            if ty != CloseType::Regular {
                shared_stream.close();
            }
            debug!("ProcessConnectionFuture terminated {:?}", ty);
            Ok(())
        }).map_err(|_| {
            error!("ProcessConnectionFuture errored");
        });

        (Self {
            inner: Box::new(connection)
        }, ClosingHelper::new(closing_tx, notifier))
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
