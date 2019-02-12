use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::poll_fn;
use futures::prelude::*;
use native_tls::{Identity, TlsAcceptor};
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio::prelude::*;
use tokio_tls::TlsAcceptor as TokioTlsAcceptor;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::stream::Stream as StreamSwitcher;
use tungstenite::stream::Mode;
use url::Url;

use network_primitives::address::PeerAddress;
use network_primitives::protocol::ProtocolFlags;
use utils::observer::PassThroughNotifier;

use crate::connection::{AddressInfo, NetworkConnection};
use crate::network_config::{NetworkConfig, ProtocolConfig};
use crate::websocket::{
    nimiq_accept_async,
    nimiq_connect_async,
    NimiqMessageStream,
    reverse_proxy::ReverseProxyCallback,
    reverse_proxy::ToCallback,
    SharedNimiqMessageStream,
    Error,
};
use crate::websocket::error::ConnectError;
use crate::websocket::error::ServerStartError;

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

pub enum WebSocketConnectorEvent {
    Connection(NetworkConnection),
    Error(Arc<PeerAddress>, ConnectError),
}

/// This function wraps a stream with a TLS certificate.
pub fn wrap_stream<S>(socket: S, tls_acceptor: Option<TlsAcceptor>, mode: Mode)
        -> Box<Future<Item=MaybeTlsStream<S>, Error=Error> + Send>
    where
        S: 'static + AsyncRead + AsyncWrite + Send,
{
    match mode {
        Mode::Plain => Box::new(future::ok(StreamSwitcher::Plain(socket))),
        Mode::Tls => {
            Box::new(future::result(tls_acceptor.ok_or(Error::TlsWrappingError))
                .map(TokioTlsAcceptor::from)
                .and_then(move |acceptor| {
                    acceptor.accept(socket)
                        .map_err(|_| Error::TlsWrappingError)
                })
                .map(|s| StreamSwitcher::Tls(s)))
        }
    }
}

/// This function loads and reads a TLS certificate.
fn setup_tls_acceptor(identity_file: Option<String>, identity_passphrase: Option<String>, mode: Mode) -> Result<Option<TlsAcceptor>, ServerStartError> {
    match mode {
        Mode::Plain => Ok(None),
        Mode::Tls => {
            let identity_file = identity_file.ok_or(ServerStartError::CertificateMissing)?;
            let identity_passphrase = identity_passphrase.ok_or(ServerStartError::CertificatePassphraseError)?;
            let mut file = File::open(identity_file).map_err(|_| ServerStartError::CertificateMissing)?;
            let mut pkcs12 = vec![];
            file.read_to_end(&mut pkcs12).map_err(|_| ServerStartError::CertificateMissing)?;
            let pkcs12 = Identity::from_pkcs12(&pkcs12,  &identity_passphrase).map_err(|_| ServerStartError::CertificatePassphraseError)?;
            Ok(Some(TlsAcceptor::new(pkcs12).map_err(|e| ServerStartError::TlsError(e))?))
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

    pub fn start(&self) -> Result<(), ServerStartError> {
        let protocol_config = self.network_config.protocol_config();

        let (port, identity_file, identity_passphrase, mode, reverse_proxy_config) = match protocol_config {
            ProtocolConfig::Ws{port, reverse_proxy_config, ..} => {
                (*port, None, None, Mode::Plain, reverse_proxy_config.clone())
            },
            ProtocolConfig::Wss{port, identity_file, identity_password, ..} => {
                (*port, Some(identity_file.to_string()), Some(identity_password.to_string()), Mode::Tls, None)
            },
            config => return Err(ServerStartError::UnsupportedProtocol(format!("{:?}", config))),
        };

        let tls_acceptor = setup_tls_acceptor(identity_file, identity_passphrase, mode)?;

        let addr = SocketAddr::new("::".parse().unwrap(), port);
        let socket = TcpListener::bind(&addr).map_err(|e| ServerStartError::IoError(e))?;
        let notifier = Arc::clone(&self.notifier);

        let srv = socket.incoming().map_err(|err| Error::IoError(err))
            .for_each(move |tcp| {
                let reverse_proxy_config = reverse_proxy_config.clone();

                let notifier = Arc::clone(&notifier);
                let acceptor = tls_acceptor.clone();
                wrap_stream(tcp, acceptor, mode).and_then(move |ss| {
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
                            tokio::spawn(poll_fn(move || shared_stream.close()).map_err(|e| {
                                warn!("Could not close connection: {}", e);
                                ()
                            }));
                        }
                    })
                })
            })
            .map_err(|err| {
                error!("Could not accept connection: {}", err);
                ()
            });

        tokio::spawn(srv);
        Ok(())
    }

    pub fn connect(&self, peer_address: Arc<PeerAddress>) -> Arc<ConnectionHandle> {
        let notifier = Arc::clone(&self.notifier);

        if !self.network_config.protocol_mask().contains(ProtocolFlags::from(peer_address.protocol())) {
            notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&peer_address), ConnectError::ProtocolMismatch));
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
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&error_peer_address), ConnectError::WebSocket(error)));
                } else if error.is_timer() {
                    let error = error.into_timer().expect("There was no timer error inside the timeout::Error struct: abort.");
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&error_peer_address), ConnectError::Timer(error)));
                } else if error.is_elapsed() {
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(Arc::clone(&error_peer_address), ConnectError::Timeout));
                }
                ()
            });

            tokio::spawn(connect);

            connection_handle
    }
}
