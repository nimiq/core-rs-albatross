use std::{
    net::SocketAddr,
    fs::File,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use url::Url;
use parking_lot::RwLock;
use native_tls::{Identity, TlsAcceptor};

use futures::{
    prelude::*,
};

use tokio::{
    io,
    prelude::*,
    net::TcpListener,
};
use tokio_tls::{TlsAcceptor as TokioTlsAcceptor};
use tungstenite::stream::Mode;

use tokio_tungstenite::{
    MaybeTlsStream,
    stream::Stream as StreamSwitcher,
};

use crate::network::{
    Protocol,
    address::PeerAddress,
    connection::{
        AddressInfo,
        NetworkConnection,
    },
    network_config::{
        NetworkConfig,
        ProtocolConfig,
    },
    websocket::{
        nimiq_connect_async,
        nimiq_accept_async,
        NimiqMessageStream,
        SharedNimiqMessageStream,
    },
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

pub fn wrap_stream<S>(socket: S, identity_file: Option<String>, mode: Mode)
        -> Box<Future<Item=MaybeTlsStream<S>, Error=io::Error> + Send>
    where
        S: 'static + AsyncRead + AsyncWrite + Send,
    {
        match mode {
            Mode::Plain => Box::new(future::ok(StreamSwitcher::Plain(socket))),
            Mode::Tls => {
                let identity_file = identity_file.expect("Tls option should always be called with a certificate.");
                let mut file = File::open(identity_file).unwrap();
                let mut pkcs12 = vec![];
                file.read_to_end(&mut pkcs12).unwrap();
                let pkcs12 = Identity::from_pkcs12(&pkcs12, "hunter2").unwrap();
                Box::new(future::result(TlsAcceptor::new(pkcs12))
                            .map(TokioTlsAcceptor::from)
                            .and_then(move |acceptor| acceptor.accept(socket))
                            .map(|s| StreamSwitcher::Tls(s))
                            .map_err(|e| io::Error::new(io::ErrorKind::Other, e)))
            }
        }
    }

pub struct WebSocketConnector {
    protocol: Protocol,
    protocol_prefix: String,
    network_config: Arc<RwLock<NetworkConfig>>,
    notifier: Arc<RwLock<Notifier<'static, WebSocketConnectorEvent>>>,
}

impl WebSocketConnector {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5); // 5 seconds

    pub fn new(protocol: Protocol, protocol_prefix: String, network_config: Arc<RwLock<NetworkConfig>>) -> WebSocketConnector {
        WebSocketConnector {
            protocol,
            protocol_prefix,
            network_config,
            notifier: Arc::new(RwLock::new(Notifier::new())),
        }
    }

    pub fn start(&self) {
        let network_config = self.network_config.read();
        let protocol_config = network_config.protocol_config();

        let (host, port, identity_file, mode) = match protocol_config {
            ProtocolConfig::Ws{host, port, reverse_proxy_config} => {
                (host.to_string(), port.clone(), None, Mode::Plain)
            },
            ProtocolConfig::Wss{host, port, identity_file} => {
                (host.to_string(), port.clone(), Some(identity_file.to_string()), Mode::Tls)
            },
            _ => panic!("Protocol not supported"),
        };

        let addr = SocketAddr::new(host.parse().expect("Invalid IP address"), port);
        let socket = TcpListener::bind(&addr).unwrap();
        let notifier = Arc::clone(&self.notifier);

        let srv = socket.incoming().for_each(move |tcp| {
            let notifier = Arc::clone(&notifier);
            let identity_file = identity_file.clone();
                wrap_stream(tcp, identity_file, mode).and_then(move |ss| {
                    nimiq_accept_async(ss).map(move |msg_stream: NimiqMessageStream| {
                        // FIXME: Find a way to provide reverse proxy support (i.e. getting the correct IP
                        // address from the request headers instead of from the socket itself)
                        let shared_stream: SharedNimiqMessageStream = msg_stream.into();
                        let net_address = Some(Arc::new(shared_stream.net_address()));
                        let (nc, ncfut) = NetworkConnection::new_connection_setup(shared_stream, AddressInfo::new(net_address, None));
                        notifier.read().notify(WebSocketConnectorEvent::Connection(nc));
                        tokio::spawn(ncfut);
                    })
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
