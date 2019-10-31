use std::convert::TryFrom;
use std::path::PathBuf;

use primitives::networks::NetworkId;

use crate::error::Error;
use crate::config::user_agent::UserAgent;
use crate::client::Client;

pub mod paths;
pub mod user_agent;


/// The default port for `ws` and `wss`.
pub const WS_DEFAULT_PORT: u16 = 8443;


/// The consensus type
///
/// # Notes
///
/// core-rs / Albatross is currently only supporting full consensus.
///
/// # ToDo
///
/// * We'll propably have this enum somewhere in the primitives. So this is a placeholder.
///
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum ConsensusConfig {
    Full,
    MacroSync,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self::Full
    }
}


/// Contains which protocol to use and the configuration needed for that protocol.
///
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum ProtocolConfig {
    /// The dumb protocol will not accept any incoming connections. This is not recommended.
    ///
    /// # Notes
    ///
    /// This is currently not supported.
    ///
    Dumb,

    /// Accept connections over an unsecure websocket. This is not recommended. Use `Wss` whenever
    /// possible.
    ///
    Ws {
        /// The hostname of your machine. This must be a valid domain name or IP address as it
        /// will be advertised to other peers in order for them to connect to you.
        ///
        host: String,

        /// The port on which Nimiq will listen for incoming connections.
        ///
        port: u16,
    },
    Wss {
        /// The hostname of your machine. This must be a valid domain name as it will be advertised
        /// to other peers in order for them to connect to you. Also this must be the CN in your
        /// SSL certificate.
        ///
        host: String,

        /// The port on which Nimiq will listen for incoming connections.
        ///
        port: u16,

        /// Path to your PKCS#12 key file, that contains private key and certificate.
        ///
        /// # Notes
        ///
        /// Only PKCS#12 is supported right now, but it is planned to move away from this and use
        /// the PEM format for certificate and private key.
        ///
        pkcs12_key_file: PathBuf,

        /// PKCS#12 is always encrypted, therefore you must provide a password for Nimiq to be able
        /// to access your SSL private key.
        ///
        pkcs12_passphrase: String,
    },

    /// Accept incoming connections over WebRTC
    ///
    /// # Notes
    ///
    /// This is currently not supported.
    ///
    Rtc,
}


#[derive(Debug, Clone)]
pub enum Feature {
    // TODO
    //RpcServer { config: (/*RpcServerConfig*/) },
    //MetricsServer { config: (/*MetricsServerConfig*/) },
    Validator /*{ config: ValidatorConfig }*/,
}

#[derive(Debug, Clone)]
pub struct ReverseProxyConfig {
    port: u16,
    header: String,
    addresses: Vec<String>,
    termination: bool,
}

#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    // TODO
}

/// Determines where the database will be stored.
///
/// # ToDo
///
///  * Implement `TryInto<FileLocations>`?
///
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum StorageConfig {
    /// This will store the database in a volatile storage. After the client shuts
    /// down all data will be lost.
    ///
    Volatile,

    /// This will store the database at the specific path. This is not available when compiled to
    /// WebAssembly.
    ///
    /// # Notes
    ///
    /// `PathBuf` must point to the **directory** in which the database should be stored.
    /// Additionally to the database the peer and validator key will also be stored in that
    /// directory.
    ///
    Path(PathBuf),

    /// This will store the database in the browser using *IndexedDB* under the specified key.
    /// This is only available when run in the browser.
    ///
    /// # Notes
    ///
    /// There no support for WebAssembly yet.
    ///
    IndexedDB(String),
}

impl StorageConfig {
    /// Stores the database in the users home directory, i.e. `$HOME/.nimiq/`. This is the default.
    ///
    pub fn home() -> Self {
        Self::Path(paths::home())
    }

    /// Stores the database in `/var/lib/nimiq/`
    pub fn system() -> Self {
        Self::Path(paths::system())
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self::home()
    }
}



#[derive(Clone, Debug, Builder)]
#[builder(setter(into), build_fn(private, name="build_internal"))]
// #[builder(pattern = "owned")]
pub struct ClientConfig {
    /// Determines which consensus protocol to use.
    ///
    /// Default is full consensus.
    ///
    #[builder(default)]
    pub consensus: ConsensusConfig,

    /// The `ProtocolConfig` that determines how the client accepts incoming connections. This
    /// will also determine how the client advertises itself to the network.
    ///
    pub protocol: ProtocolConfig,

    /// The user agent is a custom string that is sent during the handshake. Usually it contains
    /// the kind of node, Nimiq version, processor architecture and operating system. This enable
    /// gathering information on which Nimiq versions are being run on the network. A typical
    /// user agent would be `core-rs-albatross/0.1.0 (native; linux x86_64)`
    ///
    /// Default will generate a value from system information. This is recommended.
    ///
    #[builder(default)]
    pub user_agent: UserAgent,

    /// The Nimiq network the client should connect to. Usually this should be either `Test` or
    /// `Main` for the Nimiq 1.0 networks. For Albatross there is currently only `TestAlbatross`
    /// and `DevAlbatross` available. Since Albatross is still in development at time of writing,
    /// it is recommended to use `DevAlbatross`.
    ///
    /// Default is `DevAlbatross`
    ///
    #[builder(default="NetworkId::DevAlbatross")]
    pub network_id: NetworkId,

    /// Which features to use.
    ///
    #[builder(default)]
    pub features: Vec<Feature>,

    /// This configuration is needed if your node runs behind a reverse proxy.
    ///
    #[builder(setter(custom), default)]
    pub reverse_proxy_config: Option<ReverseProxyConfig>,

    /// Determines where the database is stored.
    ///
    #[builder(default)]
    pub storage: StorageConfig,
}

impl ClientConfig {
    /// Creates a new builder object for the client configuration.
    /// 
    pub fn builder() -> ClientConfigBuilder {
        ClientConfigBuilder::default()
    }

    /// Instantiates the Nimiq client from this configuration
    ///
    pub fn instantiate_client(self) -> Result<Client, Error> {
        Client::try_from(self)
    }
}



impl ClientConfigBuilder {
    /// Build a finished config object from the builder
    ///
    pub fn build(&self) -> Result<ClientConfig, Error> {
        // NOTE: We rename the generated builder and make it private to map the error from a plain
        // `String` to an actual Error.
        // We could also put some validation here.

        self.build_internal()
            .map_err(|s| Error::ConfigError(s))
    }

    /// Short cut to build the config and instantiate the client
    ///
    pub fn instantiate_client(&self) -> Result<Client, Error> {
        self.build()?
            .instantiate_client()
    }

    /// Sets the network ID to the Albatross DevNet
    ///
    pub fn dev(&mut self) -> &mut Self {
        self.network_id(NetworkId::DevAlbatross)
    }

    /// Sets the network ID to the Albatross TestNet
    ///
    pub fn test(&mut self) -> &mut Self {
        self.network_id(NetworkId::TestAlbatross)
    }

    /// Sets the client to sync the full block chain.
    ///
    pub fn full(&mut self) -> &mut Self {
        self.consensus(ConsensusConfig::Full)
    }

    /// Sets the client to sync only macro blocks util it's fully synced. Afterwards it behaves
    /// like a full node.
    ///
    pub fn macro_sync(&mut self) -> &mut Self {
        self.consensus(ConsensusConfig::MacroSync)
    }

    /// Sets the *Dumb* protocol - i.e. no incoming connections will be accepted.
    ///
    /// # Notes
    ///
    /// This is currently not supported.
    ///
    pub fn dumb(&mut self) -> &mut Self {
        self.protocol(ProtocolConfig::Dumb)
    }

    /// Sets the *Rtc* (WebRTC) protocol
    ///
    /// # Notes
    ///
    /// This is currently not supported.
    ///
    pub fn rtc(&mut self) -> &mut Self {
        self.protocol(ProtocolConfig::Rtc)
    }

    /// Sets the *Ws* (insecure Websocket) protocol.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname at which the client is accepting connections.
    /// * `port` - The port on which the client is accepting connections.
    ///
    pub fn ws<H: Into<String>, P: Into<Option<u16>>>(&mut self, host: H, port: P) -> &mut Self {
        self.protocol(ProtocolConfig::Ws {
            host: host.into(),
            port: port.into().unwrap_or(WS_DEFAULT_PORT)
        })
    }

    /// Sets the *Wss* (secure Websocket) protocol
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname at which the client is accepting connections.
    /// * `port` - The port on which the client is accepting connections.
    ///
    pub fn wss<H: Into<String>, P: Into<Option<u16>>, K: Into<PathBuf>, Q: Into<String>>(&mut self, host: H, port: P, pkcs12_key_file: K, pkcs12_passphrase: Q) -> &mut Self {
        self.protocol(ProtocolConfig::Wss {
            host: host.into(),
            port: port.into().unwrap_or(WS_DEFAULT_PORT),
            pkcs12_key_file: pkcs12_key_file.into(),
            pkcs12_passphrase: pkcs12_passphrase.into(),
        })
    }

    /// Sets the reverse proxy configuration. You need to set this if you run your node behind
    /// a reverse proxy.
    ///
    /// # Arguments
    ///
    /// * `port` - Port at which the reverse proxy is listening for incoming connections
    /// * `header` - Name of header which contains the origin IP address
    /// * `addresses` - Addresses on which the reverse proxy is listening for incoming connections
    /// * `termination` - TODO
    ///
    pub fn reverse_proxy(&mut self, port: u16, header: String, addresses: Vec<String>, termination: bool) -> &mut Self {
        self.reverse_proxy_config = Some(Some(ReverseProxyConfig {
            port,
            header,
            addresses,
            termination,
        }));
        self
    }

    /// Configure the storage to be volatile. All data will be lost after shutdown of the client.
    pub fn volatile(&mut self) -> &mut Self {
        self.storage = Some(StorageConfig::Volatile);
        self
    }
}
