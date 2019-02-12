use std::{
    fs::File,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::{
    prelude::*,
    future::poll_fn,
};
use native_tls::{Identity, TlsAcceptor};
use parking_lot::RwLock;
use tokio::{
    io,
    net::TcpListener,
    prelude::*,
};
use tokio_tls::TlsAcceptor as TokioTlsAcceptor;
use tokio_tungstenite::{
    MaybeTlsStream,
    stream::Stream as StreamSwitcher,
};
use tungstenite::stream::Mode;
use url::Url;

use network_primitives::{
    address::PeerAddress,
    protocol::ProtocolFlags,
};
use utils::observer::PassThroughNotifier;

use crate::{
    connection::{
        AddressInfo,
        NetworkConnection,
    },
    network_config::{
        NetworkConfig,
        ProtocolConfig,
    },
    websocket::{
        nimiq_accept_async,
        nimiq_connect_async,
        NimiqMessageStream,
        reverse_proxy::ReverseProxyCallback,
        reverse_proxy::ToCallback,
        SharedNimiqMessageStream,
    },
};

// This handle allows the ConnectionPool in the upper layer to signal if this
// connection should be aborted (f.e. if we are connecting to the same peer,
// once as a client and once as the server). This is equivalent to the `abort()`
// method on the WebSocketConnector on the JavaScript implementation.
pub struct ConnectionHandle(AtomicBool);

impl ConnectionHandle {
    pub fn abort(&self) {
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

pub fn wrap_stream<S>(socket: S, identity_file: Option<String>, identity_password: Option<String>, mode: Mode)
        -> Box<Future<Item=MaybeTlsStream<S>, Error=io::Error> + Send>
    where
        S: 'static + AsyncRead + AsyncWrite + Send,
{
    match mode {
        Mode::Plain => Box::new(future::ok(StreamSwitcher::Plain(socket))),
        Mode::Tls => {
            // get identity file or return error
            let identity_file = match identity_file {
                Some(i) => i,
                None => {
                    return Box::new(future::result(Err(io::Error::new(io::ErrorKind::Other, "Tls option should always be called with a certificate."))));
                }
            };
            // get identity password, defaults to 'nimiq'
            let identity_password = identity_password.unwrap_or("nimiq".into());

            // read file and return error if this fails
            let mut file = File::open(identity_file).unwrap();
            let mut pkcs12 = vec![];
            file.read_to_end(&mut pkcs12).unwrap();
            let pkcs12 = match Identity::from_pkcs12(&pkcs12, &identity_password) {
                Ok(i) => {debug!("Loaded PKCS#12 identity file"); i},
                Err(e) => {
                    return Box::new(future::result(Err(io::Error::new(io::ErrorKind::Other, e))))
                }
            };

            Box::new(future::result(TlsAcceptor::new(pkcs12))
                        .map(TokioTlsAcceptor::from)
                        .and_then(move |acceptor| acceptor.accept(socket))
                        .map(|s| StreamSwitcher::Tls(s))
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e)))
        }
    }
}

pub struct WebSocketConnector {
    network_config: Arc<NetworkConfig>,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, WebSocketConnectorEvent>>>,
}

impl WebSocketConnector {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5); // 5 seconds

    pub fn new(network_config: Arc<NetworkConfig>) -> WebSocketConnector {
        WebSocketConnector {
            network_config,
            notifier: Arc::new(RwLock::new(PassThroughNotifier::new())),
        }
    }

    pub fn start(&self) {
        let protocol_config = self.network_config.protocol_config();

        let (port, identity_file, identity_password, mode, reverse_proxy_config) = match protocol_config {
            ProtocolConfig::Ws{port, reverse_proxy_config, ..} => {
                (*port, None, None, Mode::Plain, reverse_proxy_config.clone())
            },
            ProtocolConfig::Wss{port, identity_file, identity_password, ..} => {
                (*port, Some(identity_file.to_string()), Some(identity_password.to_string()), Mode::Tls, None)
            },
            _ => panic!("Protocol not supported"),
        };

        // TODO remove unwraps. If the port is already used, this will panic
        // TODO
        let addr = SocketAddr::new("::".parse().unwrap(), port);
        let socket = TcpListener::bind(&addr).unwrap();
        let notifier = Arc::clone(&self.notifier);

        let srv = socket.incoming().for_each(move |tcp| {
            let reverse_proxy_config = reverse_proxy_config.clone();

            let notifier = Arc::clone(&notifier);

            wrap_stream(tcp, identity_file.clone(), identity_password.clone(), mode).and_then(move |ss| {
                let callback = ReverseProxyCallback::new(reverse_proxy_config.clone());
                nimiq_accept_async(ss, callback.clone().to_callback()).map(move |msg_stream: NimiqMessageStream| {
                    let mut shared_stream: SharedNimiqMessageStream = msg_stream.into();
                    // Only accept connection, if net address could be determined.
                    if let Some(net_address) = callback.check_reverse_proxy(shared_stream.net_address()) {
                        let net_address = Some(Arc::new(net_address));
                        let (nc, ncfut) = NetworkConnection::new_connection_setup(shared_stream, AddressInfo::new(net_address, None));
                        notifier.read().notify(WebSocketConnectorEvent::Connection(nc));
                        tokio::spawn(ncfut);
                    } else {
                        tokio::spawn(poll_fn(move || shared_stream.close()));
                    }
                })
            })
        })
        .map_err(|_error| (
            // FIXME: what should we do with incoming connection errors?
            ()
        ));

        tokio::spawn(srv);
    }

    pub fn connect(&self, peer_address: Arc<PeerAddress>) -> Arc<ConnectionHandle> {
        let notifier = Arc::clone(&self.notifier);

        if !self.network_config.protocol_mask().contains(ProtocolFlags::from(peer_address.protocol())) {
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
