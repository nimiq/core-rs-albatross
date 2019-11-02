use std::env;
use std::sync::Arc;

use futures::{Async, Future, Poll};

use consensus::{Consensus, ConsensusProtocol, AlbatrossConsensusProtocol, NimiqConsensusProtocol};
use database::Environment;
use mempool::MempoolConfig;
use network::network_config::{NetworkConfig, ReverseProxyConfig, Seed};
use network_primitives::address::NetAddress;
use network_primitives::protocol::Protocol;
use primitives::networks::NetworkId;
use utils::key_store::KeyStore;
use network_primitives::services::ServiceFlags;

use crate::error::ClientError;
use crate::block_producer::BlockProducer;

lazy_static! {
    pub static ref DEFAULT_USER_AGENT: String = format!("core-rs/{} (native; {} {})", env!("CARGO_PKG_VERSION"), env::consts::OS, env::consts::ARCH);
}


/// Builder for consensus and client
pub struct ClientBuilder {
    protocol: Protocol,
    environment: Environment,
    peer_key_store: KeyStore,
    hostname: Option<String>,
    port: Option<u16>,
    user_agent: String,
    network_id: NetworkId,
    reverse_proxy_config: Option<ReverseProxyConfig>,
    instant_inbound: bool,
    additional_seeds: Vec<Seed>,
    identity_file: Option<String>,
    identity_password: Option<String>,
    mempool_config: Option<MempoolConfig>,
    service_flags: Option<ServiceFlags>,
}

impl ClientBuilder {
    pub fn new(protocol: Protocol, environment: Environment, peer_key_store: KeyStore) -> Self {
        ClientBuilder {
            protocol,
            environment,
            peer_key_store,
            hostname: None,
            port: None,
            user_agent: DEFAULT_USER_AGENT.clone(),
            network_id: NetworkId::Main,
            reverse_proxy_config: None,
            instant_inbound: false,
            additional_seeds: Vec::new(),
            identity_file: None,
            identity_password: None,
            mempool_config: None,
            service_flags: None,
        }
    }

    pub fn with_network_id(&mut self, network_id: NetworkId) -> &mut Self {
        self.network_id = network_id;
        self
    }

    pub fn with_hostname(&mut self, hostname: &str) -> &mut Self {
        self.hostname = Some(String::from(hostname));
        self
    }

    pub fn with_port(&mut self, port: u16) -> &mut Self {
        self.port = Some(port);
        self
    }

    pub fn with_user_agent(&mut self, user_agent: &str) {
        self.user_agent = String::from(user_agent);
    }

    pub fn with_seeds(&mut self, seeds: Vec<Seed>) -> &mut Self {
        self.additional_seeds.extend(seeds);
        self
    }

    pub fn with_reverse_proxy(&mut self, port: u16, address: NetAddress, header: String, with_tls_termination: bool) -> &mut Self {
        self.reverse_proxy_config = Some(ReverseProxyConfig{port, address, header, with_tls_termination});
        self
    }

    pub fn with_tls_identity(&mut self, identity_file: &str, identity_password: &str) -> &mut Self {
        self.identity_file = Some(String::from(identity_file));
        self.identity_password = Some(String::from(identity_password));
        self
    }

    pub fn with_mempool_config(&mut self, mempool_config: MempoolConfig) -> &mut Self {
        self.mempool_config = Some(mempool_config);
        self
    }

    pub fn with_service_flags(&mut self, service_flags: ServiceFlags) -> &mut Self {
        self.service_flags = Some(service_flags);
        self
    }

    pub fn with_instant_inbound(&mut self, instant_inbound: bool) -> &mut Self {
        self.instant_inbound = instant_inbound;
        self
    }

    pub fn build_client<P, BP>(self, block_producer_config: BP::Config) -> Result<ClientInitializeFuture<P, BP>, ClientError>
        where P: ConsensusProtocol + 'static,
              BP: BlockProducer<P> + 'static
    {
        let consensus = self.build_consensus()?;

        Ok(ClientInitializeFuture {
            consensus,
            block_producer_config,
            initialized: false
        })
    }

    pub fn build_albatross_client<BP>(self, block_producer_config: BP::Config) -> Result<ClientInitializeFuture<AlbatrossConsensusProtocol, BP>, ClientError>
        where BP: BlockProducer<AlbatrossConsensusProtocol> + 'static
    {
        self.build_client(block_producer_config)
    }

    pub fn build_powchain_client<BP>(self, block_producer_config: BP::Config) -> Result<ClientInitializeFuture<NimiqConsensusProtocol, BP>, ClientError>
        where BP: BlockProducer<NimiqConsensusProtocol> + 'static
    {
        self.build_client(block_producer_config)
    }

    pub fn build_consensus<P: ConsensusProtocol + 'static>(self) -> Result<Arc<Consensus<P>>, ClientError> {
        // deconstruct builder
        let Self {
            environment,
            peer_key_store,
            network_id,
            mempool_config,
            protocol,
            hostname,
            port,
            reverse_proxy_config,
            instant_inbound,
            identity_file,
            identity_password,
            user_agent,
            additional_seeds,
            service_flags,
        } = self;

        // build network config
        let mut network_config = match protocol {
            Protocol::Dumb => {
                // NOTE: Just ignore hostname and port
                //if self.hostname.is_some() { return Err(ClientError::UnexpectedHostname) }
                //if self.port.is_some() { return Err(ClientError::UnexpectedPort) }
                NetworkConfig::new_dumb_network_config()
            },
            Protocol::Ws => {
                let hostname = hostname.ok_or(ClientError::MissingHostname)?;
                let port = port.unwrap_or(self.protocol.default_port()
                    .ok_or(ClientError::MissingPort)?);
                NetworkConfig::new_ws_network_config(hostname, port, instant_inbound, reverse_proxy_config)
            },
            Protocol::Wss => {
                let hostname = hostname.ok_or(ClientError::MissingHostname)?;
                let port = port.unwrap_or(protocol.default_port()
                    .ok_or(ClientError::MissingPort)?);
                let identity_file = identity_file.ok_or(ClientError::MissingIdentityFile)?;
                let identity_password = identity_password.ok_or(ClientError::MissingIdentityFile)?;
                NetworkConfig::new_wss_network_config(hostname, port, instant_inbound, identity_file, identity_password)
            },
            Protocol::Rtc => {
                return Err(ClientError::RtcNotImplemented)
            },
        };
        network_config.set_user_agent(user_agent);
        network_config.set_additional_seeds(additional_seeds);
        network_config.init_persistent(&peer_key_store)?;

        if let Some(flags) = service_flags {
            let mut services = network_config.services().clone();
            services.accepted |= flags;
            services.provided |= flags;
            network_config.set_services(services);
        }

        let mempool_config = mempool_config.unwrap_or_else(MempoolConfig::default);
        Ok(Consensus::new(environment, network_id, network_config, mempool_config)?)
    }
}

/// A trait representing a Client that may be uninitialized, initialized or connected
/// NOTE: The futures and imtermediate states implement this
pub trait Client<P>
    where P: ConsensusProtocol + 'static
{
    fn initialized(&self) -> bool;
    fn connected(&self) -> bool;
    fn consensus(&self) -> Arc<Consensus<P>>;
}


/// Future that eventually returns a InitializedClient
pub struct ClientInitializeFuture<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    consensus: Arc<Consensus<P>>,
    block_producer_config: BP::Config,
    initialized: bool
}

impl<P, BP> Future for ClientInitializeFuture<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    type Item = InitializedClient<P, BP>;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<InitializedClient<P, BP>, ClientError> {
        // NOTE: This is practically Future::fuse, but this way the types are cleaner
        if !self.initialized {
            // initialize network
            self.consensus.network.initialize().map_err(ClientError::NetworkError)?;

            // initialize block producer
            // TODO: avoid clone
            let block_producer = BP::new(self.block_producer_config.clone(), Arc::clone(&self.consensus))?;

            self.initialized = true;
            Ok(Async::Ready(InitializedClient {
                consensus: Arc::clone(&self.consensus),
                block_producer: Arc::new(block_producer),
            }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}

impl<P, BP> Client<P> for ClientInitializeFuture<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    fn initialized(&self) -> bool {
        self.initialized
    }

    fn connected(&self) -> bool {
        false
    }

    fn consensus(&self) -> Arc<Consensus<P>> {
        Arc::clone(&self.consensus)
    }
}


/// The initialized client
pub struct InitializedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    consensus: Arc<Consensus<P>>,
    block_producer: Arc<BP>,
}

impl<P, BP> InitializedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    pub fn connect(&self) -> ClientConnectFuture<P, BP> {
        ClientConnectFuture {
            consensus: Arc::clone(&self.consensus),
            block_producer: Arc::clone(&self.block_producer),
            connected: false
        }
    }

    pub fn block_producer(&self) -> Arc<BP> {
        Arc::clone(&self.block_producer)
    }
}

impl<P, BP> Client<P> for InitializedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        false
    }

    fn consensus(&self) -> Arc<Consensus<P>> {
        Arc::clone(&self.consensus)
    }
}


/// Future that eventually returns a ConnectedClient
pub struct ClientConnectFuture<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    consensus: Arc<Consensus<P>>,
    block_producer: Arc<BP>,
    connected: bool
}

impl<P, BP> Future for ClientConnectFuture<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    type Item = ConnectedClient<P, BP>;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<ConnectedClient<P, BP>, ClientError> {
        if !self.connected {
            self.consensus.network.connect().map_err(ClientError::NetworkError)?;
            self.connected = true;
            Ok(Async::Ready(ConnectedClient {
                consensus: Arc::clone(&self.consensus),
                block_producer: Arc::clone(&self.block_producer),
            }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}

impl<P, BP> Client<P> for ClientConnectFuture<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        self.connected
    }

    fn consensus(&self) -> Arc<Consensus<P>> {
        Arc::clone(&self.consensus)
    }
}


/// The connected client
pub struct ConnectedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    consensus: Arc<Consensus<P>>,
    block_producer: Arc<BP>,
}

impl<P, BP> ConnectedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    pub fn block_producer(&self) -> Arc<BP> {
        Arc::clone(&self.block_producer)
    }
}

impl<P, BP> Client<P> for ConnectedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        true
    }

    fn consensus(&self) -> Arc<Consensus<P>> {
        Arc::clone(&self.consensus)
    }
}



// TODO: Refactor!
// Too many futures! We need a client future, that never returns, to hold the consensus
// and other stuff (e.g. block producer)
impl<P, BP> Future for ConnectedClient<P, BP>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    type Item = ();
    type Error = ClientError;

    fn poll(&mut self) -> Poll<(), ClientError> {
        Ok(Async::NotReady)
    }
}
