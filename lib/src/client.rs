use std::error::Error;
use std::fmt;
use std::sync::Arc;

use failure::Fail;
use futures::{Async, Future, Poll};

use consensus::consensus::Consensus;
use consensus::error::Error as ConsensusError;
use database::Environment;
use network::error::Error as NetworkError;
use network::network::Network;
use network::network_config::{NetworkConfig, ProtocolConfig, ReverseProxyConfig};
use network_primitives::address::net_address::NetAddress;
use primitives::networks::NetworkId;

/// Builder for consensus and client
pub struct ClientBuilder {
    environment: &'static Environment,
    network_config: NetworkConfig,
    network_id: NetworkId,
    reverse_proxy_config: Option<ReverseProxyConfig>,
}

impl<'a> ClientBuilder {
    pub fn new(network_id: NetworkId, environment: &'static Environment) -> Self {
        ClientBuilder {
            environment,
            network_config: NetworkConfig::new_dumb_network_config(None),
            network_id,
            reverse_proxy_config: None,
        }
    }

    pub fn with_websocket(&mut self, host: String, port: u16, user_agent: Option<String>) -> &mut Self {
        self.network_config = NetworkConfig::new_ws_network_config(host, port, None, user_agent);
        self
    }

    pub fn with_reverse_proxy(&mut self, port: u16, address: NetAddress, header: String) -> &mut Self {
        self.reverse_proxy_config = Some(ReverseProxyConfig{port, address, header});
        self
    }

    pub fn with_websocket_secure(&mut self, host: String, port: u16, identity_file: String, identity_password: String, user_agent: Option<String>) -> &mut Self {
        self.network_config = NetworkConfig::new_wss_network_config(host, port, identity_file, identity_password, user_agent);
        self
    }

    pub fn with_protocol_dumb(&mut self, user_agent: Option<String>) -> &mut Self {
        self.network_config = NetworkConfig::new_dumb_network_config(user_agent);
        self
    }

    pub fn build_future(&mut self) -> Result<ClientInitializeFuture, ClientError> {
        let consensus = self.build_consensus()?;
        Ok(initialize(consensus.network.clone()))
    }

    pub fn build_consensus(&mut self) -> Result<Arc<Consensus>, ClientError> {
        match self.reverse_proxy_config.take() {
            Some(reverse_proxy_config) => {
                match self.network_config.protocol_config() {
                    ProtocolConfig::Ws {host, port,..} => {
                        self.network_config = NetworkConfig::new_ws_network_config(
                            host.clone(),
                            port.clone(),
                            Some(reverse_proxy_config),
                            self.network_config.user_agent().clone(),
                        ) },
                    _ => return Err(ClientError::ConfigureReverseProxyError),
                }
            },
            None => (),
        };
        self.network_config.init_persistent()?;
        Ok(Consensus::new(self.environment, self.network_id, self.network_config.clone())?)
    }
}


/// Prototype for a Error returned by these futures
/// Errors can occur, when e.g. the bind port is already used
#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "{}", _0)]
    NetworkError(#[cause] NetworkError),
    #[fail(display = "Reverse Proxy can only be configured on Ws")]
    ConfigureReverseProxyError,
    #[fail(display = "{}", _0)]
    ConsensusError(#[cause] ConsensusError),
}

impl From<NetworkError> for ClientError {
    fn from(e: NetworkError) -> Self {
        ClientError::NetworkError(e)
    }
}

impl From<ConsensusError> for ClientError {
    fn from(e: ConsensusError) -> Self {
        ClientError::ConsensusError(e)
    }
}


/// A trait representing a Client that may be uninitialized, initialized or connected
trait Client {
    fn initialized(&self) -> bool;
    fn connected(&self) -> bool;
    fn network(&self) -> Arc<Network>;
}


/// Prototype for initialize method. This could be a method of a ClientBuilder
pub fn initialize(network: Arc<Network>) -> ClientInitializeFuture {
    ClientInitializeFuture { network, initialized: false }
}


/// Future that eventually returns a InitializedClient
pub struct ClientInitializeFuture {
    network: Arc<Network>,
    initialized: bool
}

impl Future for ClientInitializeFuture {
    type Item = InitializedClient;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<InitializedClient, ClientError> {
        // NOTE: This is practically Future::fuse, but this way the types are cleaner
        if !self.initialized {
            self.network.initialize().map_err(|e| ClientError::NetworkError(e))?;
            Ok(Async::Ready(InitializedClient { network: Arc::clone(&self.network) }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}


/// The initialized client
pub struct InitializedClient {
    network: Arc<Network>
}

impl InitializedClient {
    pub fn connect(&self) -> ClientConnectFuture {
        ClientConnectFuture { network: Arc::clone(&self.network), initialized: false }
    }
}

impl Client for InitializedClient {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        false
    }

    fn network(&self) -> Arc<Network> {
        Arc::clone(&self.network)
    }
}


/// Future that eventually returns a ConnectedClient
pub struct ClientConnectFuture {
    network: Arc<Network>,
    initialized: bool
}

impl Future for ClientConnectFuture {
    type Item = ConnectedClient;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<ConnectedClient, ClientError> {
        if !self.initialized {
            self.network.connect().map_err(|e| ClientError::NetworkError(e))?;
            Ok(Async::Ready(ConnectedClient { network: Arc::clone(&self.network) }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}


/// The connected client
pub struct ConnectedClient {
    network: Arc<Network>
}

impl ConnectedClient {
    pub fn network(&self) -> Arc<Network> {
        Arc::clone(&self.network)
    }
}

impl Client for ConnectedClient {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        true
    }

    fn network(&self) -> Arc<Network> {
        Arc::clone(&self.network)
    }
}
