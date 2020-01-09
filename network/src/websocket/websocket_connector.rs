use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, Shutdown};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::oneshot;
use futures::{select, FutureExt, SinkExt};
use native_tls::{Identity, TlsAcceptor};
use parking_lot::{Mutex, RwLock};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, TcpListener};
use tokio::time::{delay_for, timeout};
use tokio::io::ErrorKind as IOErrorKind;
use tokio::sync::watch;
use tokio_tls::TlsAcceptor as TokioTlsAcceptor;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::stream::Stream as StreamSwitcher;
use tungstenite::stream::Mode;
use url::Url;

use network_primitives::address::PeerAddress;
use network_primitives::address::net_address::NetAddress;
use network_primitives::protocol::ProtocolFlags;
use utils::futures_sync::WaitGroup;
use utils::observer::PassThroughNotifier;

use crate::connection::{AddressInfo, NetworkConnection};
use crate::connection::close_type::CloseType;
use crate::network_config::{NetworkConfig, ProtocolConfig, ReverseProxyConfig};
use crate::websocket::{
    Error,
    nimiq_accept_async,
    nimiq_connect_async,
    reverse_proxy::ReverseProxyCallback,
    reverse_proxy::ToCallback,
    SharedNimiqMessageStream,
};
use crate::websocket::error::{ConnectError, ServerStartError, ServerStopError};

// This handle allows the ConnectionPool in the upper layer to signal if this
// connection should be aborted (f.e. if we are connecting to the same peer,
// once as a client and once as the server). This is equivalent to the `abort()`
// method on the WebSocketConnector on the JavaScript implementation.
pub struct ConnectionHandle {
    closing_tx: Mutex<Option<oneshot::Sender<CloseType>>>,
    closed: AtomicBool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionStatusTarget {
    Open,  // Normal operation
    Close, // Send close message and gracefully disconnect
    Kill,  // Abort connection instantly
}

impl ConnectionHandle {
    pub fn new(closing_tx: oneshot::Sender<CloseType>) -> Self {
        Self {
            closing_tx: Mutex::new(Some(closing_tx)),
            closed: AtomicBool::new(false),
        }
    }

    pub fn abort(&self, ty: CloseType) -> bool {
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

        true
    }

    pub fn is_aborted(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

pub enum WebSocketConnectorEvent {
    Connection(NetworkConnection),
    Error(Arc<PeerAddress>, ConnectError),
}

/// This function wraps a stream with a TLS certificate.
pub async fn wrap_stream<S>(socket: S, tls_acceptor: Option<TlsAcceptor>, mode: Mode)
    -> Result<MaybeTlsStream<S>, Error>
where S: 'static + AsyncRead + AsyncWrite + Send + Unpin,
{
    match mode {
        Mode::Plain => Ok(StreamSwitcher::Plain(socket)),
        Mode::Tls => match tls_acceptor {
            None => Err(Error::TlsAcceptorMissing),
            Some(tls_acceptor) => {
                let acceptor = TokioTlsAcceptor::from(tls_acceptor);
                let stream = acceptor.accept(socket).await
                    .map_err(Error::TlsWrappingError)?;
                Ok(StreamSwitcher::Tls(stream))
            },
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
            Ok(Some(TlsAcceptor::new(pkcs12)?))
        }
    }
}

pub struct WebSocketConnector {
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, WebSocketConnectorEvent>>>,
    network_config: Arc<NetworkConfig>,
    port: u16,
    mode: Mode,
    reverse_proxy_config: Option<ReverseProxyConfig>,
    tls_acceptor: Option<TlsAcceptor>,
    close_signal: Mutex<Option<oneshot::Sender<Duration>>>,
}

impl WebSocketConnector {
    const CONNECTIONS_MAX: usize = 4050; // A little more than Network.PEER_COUNT_MAX to allow for inbound exchange
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    const WAIT_TIME_ON_ERROR: Duration = Duration::from_millis(100);

    pub fn new(network_config: Arc<NetworkConfig>) -> Result<Arc<WebSocketConnector>, ServerStartError> {
        let protocol_config = network_config.protocol_config();

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

        Ok(Arc::new(WebSocketConnector {
            notifier: Arc::new(RwLock::new(PassThroughNotifier::new())),
            network_config,
            port, mode, reverse_proxy_config,
            tls_acceptor,
            close_signal: Mutex::new(None),
        }))
    }

    // Starts accepting and handling WebSocket connections over Tokio TCP.
    pub async fn start(self: &Arc<WebSocketConnector>) -> Result<(), ServerStartError> {
        // TODO Allow binding to IP
        let addr = SocketAddr::new("::".parse().unwrap(), self.port);
        let socket = TcpListener::bind(&addr)
            .await
            .map_err(ServerStartError::IoError)?;
        let (tx, rx) = oneshot::channel();
        *self.close_signal.lock() = Some(tx);
        tokio::spawn(Self::accept_tcp_streams(Arc::clone(&self), socket, rx));
        Ok(())
    }

    // Tries to gracefully close all connections.
    // After timeout, the remaining connections are killed.
    pub fn stop(self: &Arc<WebSocketConnector>, timeout: Duration) -> Result<(), ServerStopError> {
        match self.close_signal.lock().take() {
            Some(closer) => closer.send(timeout).expect("Failed to close server"),
            None => return Err(ServerStopError::AlreadyStopped),
        };
        Ok(())
    }

    // Accepts incoming connections and spawns connection handlers.
    // The shutdown signal makes the server stop accepting conns
    // and tells the handlers to close their connections gracefully.
    // After the duration sent in shutdown passes,
    // handlers will force-exit by dropping the TCP sessions.
    // Finally, the future completes when all connection handlers exit.
    async fn accept_tcp_streams(self: Arc<WebSocketConnector>, mut socket: TcpListener, shutdown: oneshot::Receiver<Duration>) {
        let mut shutdown = shutdown.fuse();
        let conn_count = Arc::new(Mutex::new(0usize));
        let wg = WaitGroup::new();
        let (status_tx, status_rx) = watch::channel(ConnectionStatusTarget::Open);
        let kill_timeout;
        loop {
            // Skip if we cannot accept a new connection
            {
                let mut conns = conn_count.lock();
                if *conns >= Self::CONNECTIONS_MAX {
                    continue;
                }
                *conns += 1;
            }
            // Try to accept next connection
            let conn_res = select! {
                res = shutdown => {
                    if let Ok(dur) = res {
                        kill_timeout = dur;
                    } else {
                        // Sender died, implicit kill
                        return;
                    }
                    break;
                },
                res = socket.accept().fuse() => res,
            };
            // Handle accept errors like tk-listen.
            // Connection errors should be ignored because they have no effect on the server.
            // Ignoring socket errors can cause the server to spin though,
            // we pause the server for a short time to allow the kernel to recover.
            let (tcp, addr) = match conn_res {
                Ok(conn) => conn,
                Err(err) => {
                    match err.kind() {
                        IOErrorKind::ConnectionRefused
                        | IOErrorKind::ConnectionAborted
                        | IOErrorKind::ConnectionReset => {
                            warn!("Cannot accept broken connection: {}", err);
                        },
                        _ => {
                            error!("Failed to accept TCP connection, throttling: {}", err);
                            delay_for(Self::WAIT_TIME_ON_ERROR).await;
                        }
                    }
                    continue;
                }
            };
            let this = Arc::clone(&self);
            let mut on_close = status_rx.clone();
            let wg2 = wg.clone();
            let conn_count2 = Arc::clone(&conn_count);
            tokio::spawn(async move {
                let on_close2 = on_close.clone();
                let mut on_kill = async move {
                    while let Some(status) = on_close.recv().await {
                        if status == ConnectionStatusTarget::Kill {
                            break;
                        }
                    }
                }.boxed().fuse();
                let stream_result = select! {
                    res = this.handle_tcp_stream(tcp, addr, on_close2).fuse() => res,
                    _ = on_kill => return, // Drop the TcpStream
                };
                if let Err(err) = stream_result {
                    error!("Could not accept connection: {}", err);
                }
                *conn_count2.lock() -= 1;
                // Capture the WaitGroup handle and drop it when the connection handler exits.
                drop(wg2);
            });
        }
        // Tell all connection handlers to close connection.
        if status_tx.broadcast(ConnectionStatusTarget::Close).is_err() {
            panic!("Failed to broadcast close signal to connection handlers")
        }
        // Wait for connection handlers to close connection or send kill signal on timeout.
        let mut finish_fut = wg.wait().boxed().fuse();
        select! {
            _ = finish_fut => (),
            _ = delay_for(kill_timeout).fuse() => {
                // Send kill signal and continue to wait
                if status_tx.broadcast(ConnectionStatusTarget::Kill).is_err() {
                    panic!("Failed to broadcast kill signal to connection handlers")
                }
                finish_fut.await;
            }
        }
    }

    // Handles a single WebSocket connection over tokio-tcp.
    // As soon as is_open changes to "false", the connection is closed.
    // Only the first value received by is_open may be Open.
    async fn handle_tcp_stream(
        self: &Arc<WebSocketConnector>,
        tcp: TcpStream,
        addr: SocketAddr,
        mut status: watch::Receiver<ConnectionStatusTarget>
    ) -> Result<(), Error> {
        // Before doing anything, check whether channel is open.
        if status.recv().await != Some(ConnectionStatusTarget::Open) {
            trace!("Server closing before handshake with {} could complete", addr);
            if let Err(err) = tcp.shutdown(Shutdown::Both) {
                warn!("Failed to shutdown connection to {}: {}", addr, err);
            }
            return Ok(())
        }

        let peer_addr = addr.ip();
        let net_addr = match peer_addr {
            IpAddr::V4(v) => NetAddress::IPv4(v),
            IpAddr::V6(v) => NetAddress::IPv6(v),
        };
        let notifier = Arc::clone(&self.notifier);
        let acceptor = self.tls_acceptor.clone();

        // Do TLS handshake if required
        let wrapped = wrap_stream(tcp, acceptor, self.mode)
            .await?;

        // Do Nimiq handshake and start relaying messages
        let callback = ReverseProxyCallback::new(self.reverse_proxy_config.clone());
        let msg_stream = nimiq_accept_async(wrapped, net_addr, callback.clone().to_callback())
            .await?;
        let mut shared_stream: SharedNimiqMessageStream = msg_stream.into();
        // Only accept connection, if net address could be determined.
        if let Some(net_address) = callback.check_reverse_proxy(net_addr) {
            let net_address = Some(Arc::new(net_address));
            let (nc, ncfut) = NetworkConnection::new_connection_setup(shared_stream, AddressInfo::new(net_address, None));
            notifier.read().notify(WebSocketConnectorEvent::Connection(nc));
            ncfut.await;
        } else {
            shared_stream.close().await?;
        }
        Ok(())
    }

    pub fn connect(&self, peer_address: Arc<PeerAddress>) -> Result<Arc<ConnectionHandle>, ConnectError> {
        let notifier = Arc::clone(&self.notifier);

        if !self.network_config.protocol_mask().contains(ProtocolFlags::from(peer_address.protocol())) {
            return Err(ConnectError::ProtocolMismatch);
        }

        // NOTE: We're not checking if we are already connecting to a peer here because
        // that check is already done by the ConnectionPool in the upper layer and doing
        // it here would imply creating data structures just for that (unlike the JavaScript
        // implementation where the data structures are there for something else and then you
        // get this check "for free")

        let url = Url::parse(&peer_address.as_uri().to_string()).map_err(ConnectError::InvalidUri)?;
        let error_notifier = Arc::clone(&self.notifier);
        let error_peer_address = Arc::clone(&peer_address);
        let (tx, rx) = oneshot::channel::<CloseType>();
        let connection_handle = Arc::new(ConnectionHandle::new(tx));

        let connect = async move {
            let msg_stream_res = timeout(Self::CONNECT_TIMEOUT, nimiq_connect_async(url)).await;
            match msg_stream_res {
                Ok(Ok(msg_stream)) => {
                    let shared_stream: SharedNimiqMessageStream = msg_stream.into();
                    let net_address = Some(Arc::new(shared_stream.net_address()));
                    let (nc, ncfut) = NetworkConnection::new_connection_setup(shared_stream, AddressInfo::new(net_address, Some(peer_address)));
                    notifier.read().notify(WebSocketConnectorEvent::Connection(nc));
                    tokio::spawn(ncfut);
                },
                Ok(Err(error)) => {
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(error_peer_address.clone(), error.into()));
                },
                Err(_elapsed) => {
                    error_notifier.read().notify(WebSocketConnectorEvent::Error(error_peer_address.clone(), ConnectError::Timeout));
                },
            };
        };
        tokio::spawn(async move {
            select! {
                _ = connect.fuse() => (),
                _ = rx.fuse() => ()
            }
        });

        Ok(connection_handle)
    }
}
