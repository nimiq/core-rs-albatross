use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use url::Url;
use parking_lot::RwLock;

use futures::{
    prelude::*,
};

use tokio::{
    io,
    prelude::*,
    net::TcpListener,
};

use crate::network::{
    Protocol,
    address::{PeerAddress},
    connection::{
        AddressInfo,
        NetworkConnection,
    },
    websocket::{nimiq_connect_async, nimiq_accept_async, SharedNimiqMessageStream},
};

use crate::utils::observer::Notifier;

// This handle allows the ConnectionPool in the upper layer to signal if this
// connection should be aborted (f.e. if we are connecting to the same peer,
// once as a client and once as the server). This is equivalent to the `abort()`
// method on the WebSocketConnector on the JavaScript implementation.
pub struct ConnectionHandle(AtomicBool);

impl ConnectionHandle {
    pub fn abort(&mut self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}


#[derive(Clone)]
pub enum WebSocketConnectorEvent {
    Connection(NetworkConnection),
    Error(Arc<PeerAddress>, io::ErrorKind),
}

pub struct WebSocketConnector {
    protocol: Protocol,
    protocol_prefix: String,
    network_config: String,
    notifier: Arc<RwLock<Notifier<'static, WebSocketConnectorEvent>>>,
}

impl WebSocketConnector {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5); // 5 seconds

    pub fn new(protocol: Protocol, protocol_prefix: String, network_config: String) -> WebSocketConnector {
        WebSocketConnector {
            protocol,
            protocol_prefix,
            network_config,
            notifier: Arc::new(RwLock::new(Notifier::new())),
        }
    }

    pub fn start(&self) {
        let addr = "127.0.0.1:8081".to_string().parse().unwrap();
        let notifier = Arc::clone(&self.notifier);

        let socket = TcpListener::bind(&addr).unwrap();
        let srv = socket.incoming().for_each(move |stream| {
            let notifier = Arc::clone(&notifier);
            nimiq_accept_async(stream).map(move |msg_stream| {

                // FIXME: Find a way to provide reverse proxy support (i.e. getting the correct IP
                // address from the request headers instead of from the socket itself)

                let shared_stream: SharedNimiqMessageStream = msg_stream.into();
                let net_address = Some(Arc::new(shared_stream.net_address()));
                let (nc, ncfut) = NetworkConnection::new_connection_setup(shared_stream, AddressInfo::new(net_address, None));
                notifier.read().notify(WebSocketConnectorEvent::Connection(nc));

                tokio::spawn(ncfut);
            })
        })
        .map_err(|error| (
            // FIXME: what should we do with incoming connection errors?
            ()
        ));

        tokio::spawn(srv);
    }

    pub fn connect(&self, peer_address: Arc<PeerAddress>) -> Arc<ConnectionHandle> {
        let notifier = Arc::clone(&self.notifier);

        if self.protocol != peer_address.protocol() {
            notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&peer_address), io::ErrorKind::InvalidInput));
        }

        // NOTE: We're not checking if we are already connecting to a peer here because
        // that check is already done by the ConnectionPool in the upper layer and doing
        // it here would imply creating data structures just for that (unlike the JavaScript
        // implementation where the data structures are there for something else and then you
        // get this check "for free")

        let url = Url::parse(&peer_address.as_uri()).unwrap();
        let error_notifier = Arc::clone(&self.notifier);
        let error_peer_address = Arc::clone(&peer_address);
        let connection_handle = Arc::new(ConnectionHandle(AtomicBool::new(false)));
        let connection_handle_for_closure = Arc::clone(&connection_handle);

        let connect = nimiq_connect_async(url)
            .timeout(Self::CONNECT_TIMEOUT)
            .map(move |msg_stream| {
                if !connection_handle_for_closure.is_aborted() {
                    let shared_stream: SharedNimiqMessageStream = msg_stream.into();
                    let net_address = Some(Arc::new(shared_stream.net_address()));
                    let (nc, ncfut) = NetworkConnection::new_connection_setup(shared_stream, AddressInfo::new(net_address, Some(peer_address)));
                    notifier.read().notify(WebSocketConnectorEvent::Connection(nc));
                    tokio::spawn(ncfut);
                }
            })
            .map_err(move |error| {
                if error.is_inner() {
                    let error = error.into_inner().expect("There was no inner_error inside the timeout::Error struct: abort.");
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&error_peer_address), error.kind()));
                } else if error.is_timer() {
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&error_peer_address), io::ErrorKind::Other));
                } else if error.is_elapsed() {
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&error_peer_address), io::ErrorKind::TimedOut));
                }
                ()
            });

            tokio::spawn(connect);

            connection_handle
    }
}
