use std::convert::TryFrom;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::net::IpAddr;

use derive_builder::Builder;
use enum_display_derive::Display;
use url::Url;

#[cfg(feature="validator")]
use bls::SecureGenerate;
#[cfg(feature="validator")]
use bls::bls12_381::KeyPair as BlsKeyPair;
use database::Environment;
use database::lmdb::{LmdbEnvironment, open as LmdbFlags};
use database::volatile::VolatileEnvironment;
use mempool::filter::Rules as MempoolRules;
use mempool::MempoolConfig;
use network::network_config::{NetworkConfig, ReverseProxyConfig, Seed};
use network_primitives::address::{NetAddress, SeedList, PeerUri};
use primitives::networks::NetworkId;
use utils::key_store::Error as KeyStoreError;
use utils::key_store::KeyStore;
use keys::PublicKey;

use crate::client::Client;
use crate::config::command_line::CommandLine;
use crate::config::config_file;
use crate::config::config_file::ConfigFile;
use crate::config::consts;
use crate::config::paths;
use crate::config::user_agent::UserAgent;
use crate::error::Error;


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
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Display)]
pub enum ConsensusConfig {
    Full,
    MacroSync,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self::Full
    }
}


#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PKCS12Credentials {
    /// Path to your PKCS#12 key file, that contains private key and certificate.
    ///
    /// # Notes
    ///
    /// Only PKCS#12 is supported right now, but it is planned to move away from this and use
    /// the PEM format for certificate and private key.
    ///
    pub key_file: PathBuf,

    /// PKCS#12 is always encrypted, therefore you must provide a password for Nimiq to be able
    /// to access your SSL private key.
    ///
    pub passphrase: String,
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

        /// The PKCS#12 key file and its password
        tls_credentials: PKCS12Credentials,
    },

    /// Accept incoming connections over WebRTC
    ///
    /// # Notes
    ///
    /// This is currently not supported.
    ///
    Rtc,
}

#[cfg(feature="validator")]
#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    // TODO
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct FileStorageConfig {
    /// The parent directory where the database will be stored. The database directory name
    /// is determined by the network ID and consensus type using the `database_name` static
    /// method.
    database_parent: PathBuf,

    /// Path to peer key
    peer_key: PathBuf,

    /// Path to validator key
    validator_key: Option<PathBuf>,
}

impl FileStorageConfig {
    /// Create storage config from a directory path.
    ///
    pub fn from_directory<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        Self {
            database_parent: path.to_path_buf(),
            peer_key: path.join("peer_key.dat"),
            validator_key: Some(path.join("validator_key.dat")),
        }
    }

    /// Stores the database in the users home directory, i.e. `$HOME/.nimiq/`. This is the default.
    ///
    pub fn home() -> Self {
        Self::from_directory(paths::home())
    }

    /// Stores the database in `/var/lib/nimiq/`
    pub fn system() -> Self {
        Self::from_directory(paths::system())
    }
}

impl Default for FileStorageConfig {
    fn default() -> Self {
        Self::home()
    }
}

/// Configuration options for the database
#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct DatabaseConfig {
    /// Initial database size. Default: 50 MB
    #[builder(default="50 * 1024 * 1024")]
    size: usize,

    /// Max number of DBs. Recommended: 10
    #[builder(default="10")]
    max_dbs: u32,

    /// Additional LMDB flags
    #[builder(default="LmdbFlags::NOMETASYNC")]
    flags: LmdbFlags::Flags
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            size: 50 * 1024 * 1024,
            max_dbs: 10,
            flags: LmdbFlags::NOMETASYNC,
        }
    }
}

impl From<config_file::DatabaseSettings> for DatabaseConfig {
    fn from(db_settings: config_file::DatabaseSettings) -> Self {
        let default = DatabaseConfig::default();

        let mut flags = LmdbFlags::NOMETASYNC;
        if db_settings.no_lmdb_sync.unwrap_or_default() {
            flags |= LmdbFlags::NOSYNC;
        }

        Self {
            size: db_settings.size.unwrap_or(default.size),
            max_dbs: db_settings.max_dbs.unwrap_or(default.max_dbs),
            flags,
        }
    }
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

    /// This will store the database and key files at specific paths. This is not available when
    /// compiled to WebAssembly.
    ///
    Filesystem(FileStorageConfig),

    /// This will store the database in the browser using *IndexedDB*, and the key files in
    /// *LocalStorage*. This is only available when run in the browser.
    ///
    /// # Notes
    ///
    /// There no support for WebAssembly yet.
    ///
    Browser,
}

impl StorageConfig {
    /// Returns the database environment for that storage backend and the given network ID and
    /// consensus type.
    ///
    /// # Arguments
    ///
    /// * `network_id` - The network ID of the database
    /// * `consensus` - The consensus type
    ///
    /// # Return Value
    ///
    /// Returns a `Result` which is either a `Environment` or a `Error`.
    ///
    pub fn database(&self, network_id: NetworkId, consensus: ConsensusConfig, db_config: DatabaseConfig) -> Result<Environment, Error> {
        let db_name = format!("{}-{}-consensus", network_id, consensus).to_lowercase();
        info!("Opening database: {}", db_name);

        Ok(match self {
            StorageConfig::Volatile => {
                VolatileEnvironment::new_with_lmdb_flags(db_config.max_dbs, db_config.flags)?
            },
            StorageConfig::Filesystem(file_storage) => {
                let db_path = file_storage.database_parent.join(db_name);
                let db_path = db_path.to_str()
                    .ok_or_else(|| Error::config_error(format!("Failed to convert database path to string: {}", db_path.display())))?
                    .to_string();
                LmdbEnvironment::new(&db_path, db_config.size, db_config.max_dbs, db_config.flags)?
            },
            _ => return Err(self.not_available()),
        })
    }

    pub(crate) fn init_key_store(&self, network_config: &mut NetworkConfig) -> Result<(), Error> {
        // TODO: Move this out of here and load keys from database
        match self {
            StorageConfig::Volatile => {
                network_config.init_volatile()
            },
            StorageConfig::Filesystem(file_storage) => {
                let key_path = file_storage.peer_key.to_str()
                    .ok_or_else(|| Error::config_error(format!("Failed to convert path of peer key to string: {}", file_storage.peer_key.display())))?
                    .to_string();
                let key_store = KeyStore::new(key_path.clone());
                network_config.init_persistent(&key_store)
                    .map_err(|e| {
                        warn!("Failed to initialize network config: {}", e);
                        warn!("Does the peer key file exist? {}", key_path);
                        e
                    })?;
            }
            _ => return Err(self.not_available()),
        }
        Ok(())
    }

    #[cfg(feature="validator")]
    pub(crate) fn validator_key(&self) -> Result<BlsKeyPair, Error> {
        Ok(match self {
            StorageConfig::Volatile => {
                BlsKeyPair::generate_default_csprng()
            },
            StorageConfig::Filesystem(file_storage) => {
                let key_path = file_storage.validator_key.as_ref()
                    .ok_or_else(|| Error::config_error("No path for validator key specified"))?;
                let key_path = key_path.to_str()
                    .ok_or_else(|| Error::config_error(format!("Failed to convert path of validator key to string: {}", key_path.display())))?
                    .to_string();
                let key_store = KeyStore::new(key_path);
                match key_store.load_key() {
                    Err(KeyStoreError::IoError(_)) => {
                        let validator_key = BlsKeyPair::generate_default_csprng();
                        key_store.save_key(&validator_key)?;
                        Ok(validator_key)
                    },
                    res => res,
                }?
            },
            _ => return Err(self.not_available()),
        })
    }

    fn not_available(&self) -> Error {
        Error::Config(format!("Storage backend not implemented: {:?}", self))
    }
}

impl From<FileStorageConfig> for StorageConfig {
    fn from(config: FileStorageConfig) -> Self {
        StorageConfig::Filesystem(config)
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        FileStorageConfig::default().into()
    }
}

/// Credentials for JSON RPC server, metrics server or websocket RPC server
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    /// Username
    pub username: String,
    /// Password
    pub password: String,
}

impl Credentials {
    pub fn new<U: AsRef<str>, P: AsRef<str>>(username: U, password: P) -> Self {
        Self {
            username: username.as_ref().to_owned(),
            password: password.as_ref().to_owned(),
        }
    }

    pub fn check<U: AsRef<str>, P: AsRef<str>>(&self, username: U, password: P) -> bool {
        self.username == username.as_ref() && self.password == password.as_ref()
    }
}

#[cfg(feature="rpc-server")]
#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct RpcServerConfig {
    /// Bind the RPC server to the specified IP address.
    ///
    /// Default: `127.0.0.1`
    ///
    #[builder(setter(strip_option))]
    pub bind_to: Option<IpAddr>,

    /// Bind the server to the specified port.
    ///
    /// Default: `8648`
    ///
    #[builder(default="consts::RPC_DEFAULT_PORT")]
    pub port: u16,

    /// TODO
    #[builder(setter(strip_option))]
    pub corsdomain: Option<Vec<String>>,

    /// If specified, only allow connections from these IP addresses
    ///
    #[builder(setter(strip_option))]
    pub allow_ips: Option<Vec<IpAddr>>,

    /// If specified, only allow these RPC methods
    ///
    #[builder(setter(strip_option))]
    pub allowed_methods: Option<Vec<String>>,

    /// If specified, require HTTP basic auth with these credentials
    #[builder(setter(strip_option))]
    pub credentials: Option<Credentials>,
}

#[cfg(feature="ws-rpc-server")]
#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct WsRpcServerConfig {
    /// Bind the Websocket RPC server to the specified IP address.
    ///
    /// Default: `127.0.0.1`
    ///
    #[builder(setter(strip_option))]
    pub bind_to: Option<IpAddr>,

    /// Bind the server to the specified port.
    ///
    /// Default: `8648`
    ///
    #[builder(default="consts::WS_RPC_DEFAULT_PORT")]
    pub port: u16,

    /// If specified, require HTTP basic auth with these credentials
    #[builder(setter(strip_option))]
    pub credentials: Option<Credentials>,
}

#[cfg(feature="metrics-server")]
#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct MetricsServerConfig {
    /// Bind the metrics server to the specified IP address.
    ///
    /// Default: `127.0.0.1`
    ///
    #[builder(setter(strip_option))]
    pub bind_to: Option<IpAddr>,

    /// Bind the server to the specified port.
    ///
    /// Default: `8649`
    ///
    #[builder(default="consts::METRICS_DEFAULT_PORT")]
    pub port: u16,

    /// If specified, require HTTP basic auth with these credentials
    #[builder(setter(strip_option))]
    pub credentials: Option<Credentials>,

    /// If specified, use this to encrypt the metrics server
    pub tls_credentials: Option<PKCS12Credentials>,
}

/// Client configuration
///
/// # ToDo
///
/// * Make this implement `IntoFuture<Item=Client, Err=Error>` so you can just do
///   `tokio::spawn(config.and_then(|client| [...]));`
#[derive(Clone, Debug, Builder)]
#[builder(setter(into), build_fn(private, name="build_internal"))]
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
    pub network: NetworkId,

    /// This configuration is needed if your node runs behind a reverse proxy.
    ///
    #[builder(setter(custom), default)]
    pub reverse_proxy: Option<ReverseProxyConfig>,

    /// Determines where the database is stored.
    ///
    #[builder(default)]
    pub storage: StorageConfig,

    /// Database-specific configuration
    ///
    #[builder(default)]
    pub database: DatabaseConfig,

    /// The mempool filter rules
    ///
    #[builder(default, setter(custom))]
    pub mempool: MempoolConfig,

    /// Custom seeds
    ///
    #[builder(setter(custom), default)]
    pub seeds: Vec<Seed>,

    /// The optional validator configuration
    ///
    #[cfg(feature="validator")]
    #[builder(default, setter(custom))]
    pub validator: Option<ValidatorConfig>,

    /// The optional validator configuration
    ///
    #[cfg(feature="rpc-server")]
    #[builder(default)]
    pub rpc_server: Option<RpcServerConfig>,

    /// The optional Websocket RPC configuration
    ///
    #[cfg(feature="ws-rpc-server")]
    #[builder(default)]
    pub ws_rpc_server: Option<WsRpcServerConfig>,

    /// The optional metrics server configuration
    ///
    #[cfg(feature="metrics-server")]
    #[builder(default)]
    pub metrics_server: Option<MetricsServerConfig>,
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
            .map_err(Error::config_error)
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
        self.network(NetworkId::DevAlbatross)
    }

    /// Sets the network ID to the Albatross TestNet
    ///
    pub fn test(&mut self) -> &mut Self {
        self.network(NetworkId::TestAlbatross)
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
            port: port.into().unwrap_or(consts::WS_DEFAULT_PORT)
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
            port: port.into().unwrap_or(consts::WS_DEFAULT_PORT),
            tls_credentials: PKCS12Credentials {
                key_file: pkcs12_key_file.into(),
                passphrase: pkcs12_passphrase.into(),
            },
        })
    }

    /// Sets the reverse proxy configuration. You need to set this if you run your node behind
    /// a reverse proxy.
    ///
    /// # Arguments
    ///
    /// * `port` - Port at which the reverse proxy is listening for incoming connections
    /// * `header` - Name of header which contains the origin IP address
    /// * `address` - Address on which the reverse proxy is listening for incoming connections
    /// * `termination` - TODO
    ///
    pub fn reverse_proxy(&mut self, port: u16, header: String, address: NetAddress, with_tls_termination: bool) -> &mut Self {
        self.reverse_proxy = Some(Some(ReverseProxyConfig {
            port,
            header,
            address,
            with_tls_termination,
        }));
        self
    }

    /// Configures the storage to be volatile. All data will be lost after shutdown of the client.
    pub fn volatile(&mut self) -> &mut Self {
        self.storage = Some(StorageConfig::Volatile);
        self
    }

    /// Sets the mempool filter rules
    pub fn mempool(&mut self, filter_rules: MempoolRules, filter_limit: usize) -> &mut Self {
        self.mempool = Some(MempoolConfig { filter_rules, filter_limit });
        self
    }

    /// Adds a custom seed node or seed list
    pub fn seed<S: Into<Seed>>(&mut self, seed: S) -> &mut Self {
        let seeds = self.seeds.get_or_insert_with(Default::default);
        seeds.push(seed.into());
        self
    }

    /// Adds a custom seed node
    pub fn seed_uri<U: Into<PeerUri>>(&mut self, uri: U) -> &mut Self {
        self.seed(Seed::new_peer(uri.into()))
    }

    /// Adds a custom seed url
    pub fn seed_list(&mut self, url: Url, public_key_opt: Option<PublicKey>) -> &mut Self {
        self.seed(Seed::new_list(SeedList::new(url, public_key_opt)))
    }

    /// Sets the validator config. Since there is no configuration for validators (except key file)
    /// yet, this will just enable the validator.
    #[cfg(feature="validator")]
    pub fn validator(&mut self) -> &mut Self {
        self.validator = Some(Some(ValidatorConfig {}));
        self
    }

    /// Applies settings from a configuration file
    pub fn config_file(&mut self, config_file: &ConfigFile) -> Result<&mut Self, Error> {
        // Configure protocol
        self.protocol(match config_file.network.protocol {
            config_file::Protocol::Dumb => ProtocolConfig::Dumb,
            config_file::Protocol::Ws => ProtocolConfig::Ws {
                host: config_file.network.host.clone()
                    .ok_or_else(|| Error::config_error("Hostname not set."))?
                    .clone(),
                port: config_file.network.port.clone()
                    .unwrap_or(consts::WS_DEFAULT_PORT),
            },
            config_file::Protocol::Wss => {
                let tls = config_file.network.tls.as_ref()
                    .ok_or_else(|| Error::config_error("[tls] section missing."))?
                    .clone();
                ProtocolConfig::Wss {
                    host: config_file.network.host.clone()
                        .ok_or_else(|| Error::config_error("Hostname not set."))?,
                    port: config_file.network.port.clone()
                        .unwrap_or(consts::WS_DEFAULT_PORT),
                    tls_credentials: PKCS12Credentials {
                        key_file: PathBuf::from(tls.identity_file),
                        passphrase: tls.identity_password,
                    },
                }
            },
            config_file::Protocol::Rtc => ProtocolConfig::Rtc,
        });

        // Configure user agent
        config_file.network.user_agent.as_ref()
            .map(|user_agent| self.user_agent(user_agent.clone()));

        // Configure consensus
        self.consensus(config_file.consensus.consensus_type);

        // Configure network
        self.network(config_file.consensus.network);

        // Configure storage config.
        let mut file_storage = FileStorageConfig::default();
        config_file.database.path.as_ref()
            .map(|path| {
                file_storage.database_parent = PathBuf::from(path);
            });
        config_file.peer_key_file.as_ref()
            .map(|path| {
                file_storage.peer_key = PathBuf::from(path);
            });
        config_file.validator.as_ref()
            .map(|validator_config| {
                validator_config.key_file.as_ref()
                    .map(|key_path| file_storage.validator_key = Some(PathBuf::from(key_path)));
            });
        self.storage = Some(file_storage.into());

        // Configure database
        self.database(config_file.database.clone());

        // Configure reverse proxy config
        config_file.reverse_proxy.as_ref()
            .map(|reverse_proxy| {
                self.reverse_proxy = Some(Some(reverse_proxy.clone().into()));
            });

        // Configure RPC server
        #[cfg(feature="rpc-server")] {
            if let Some(rpc_config) = &config_file.rpc_server {
                let bind_to = rpc_config.bind.as_ref()
                    .and_then(|addr| addr.into_ip_address());

                let allow_ips = if rpc_config.allowip.is_empty() {
                    None
                }
                else {
                    let result = rpc_config.allowip.iter().map(|s| {
                        s.parse::<IpAddr>().map_err({
                            |e| Error::config_error(format!("Invalid IP: {}", e))
                        })
                    }).collect::<Result<Vec<IpAddr>, Error>>();
                    Some(result?)
                };

                let credentials = match (&rpc_config.username, &rpc_config.password) {
                    (Some(u), Some(p)) => {
                        Some(Credentials::new(u.clone(), p.clone()))
                    },
                    (None, None) => None,
                    _ => return Err(Error::config_error("Either both username and password have to be set or none."))
                };

                self.rpc_server = Some(Some(RpcServerConfig {
                    bind_to,
                    port: rpc_config.port.unwrap_or(consts::RPC_DEFAULT_PORT),
                    corsdomain: Some(rpc_config.corsdomain.clone()),
                    allow_ips,
                    allowed_methods: Some(rpc_config.methods.clone()),
                    credentials,
                }));
            }
        }

        // Configure Websocket RPC server
        #[cfg(feature="ws-rpc-server")] {
            if let Some(ws_rpc_config) = &config_file.ws_rpc_server {
                let bind_to = ws_rpc_config.bind.as_ref()
                    .and_then(|addr| addr.into_ip_address());

                let credentials = match (&ws_rpc_config.username, &ws_rpc_config.password) {
                    (Some(u), Some(p)) => {
                        Some(Credentials::new(u.clone(), p.clone()))
                    },
                    (None, None) => None,
                    _ => return Err(Error::config_error("Either both username and password have to be set or none."))
                };

                self.ws_rpc_server = Some(Some(WsRpcServerConfig {
                    bind_to,
                    port: ws_rpc_config.port.unwrap_or(consts::WS_RPC_DEFAULT_PORT),
                    credentials,
                }));
            }
        }

        // Configure metrics server
        #[cfg(feature="metrics-server")] {
            if let Some(metrics_config) = &config_file.metrics_server {
                let bind_to = metrics_config.bind.as_ref()
                    .and_then(|addr| addr.into_ip_address());

                let credentials = metrics_config.password.as_ref().map(|password| {
                    Credentials::new("metrics", password)
                });

                self.metrics_server = Some(Some(MetricsServerConfig {
                    bind_to,
                    port: metrics_config.port.unwrap_or(consts::METRICS_DEFAULT_PORT),
                    credentials,
                    tls_credentials: None, // TODO
                }));
            }
        }

        // Configure custom seeds
        for seed in &config_file.network.seed_nodes {
            self.seed(Seed::try_from(seed.clone())
                .map_err(|e| Error::config_error(format!("Invalid seed: {:?}: {}", seed, e)))?);
        }

        // Configure validator
        #[cfg(feature="validator")] {
            if config_file.validator.is_some() {
                self.validator = Some(Some(ValidatorConfig {}));
            }
        }

        Ok(self)
    }

    /// Applies settings from the command line
    pub fn command_line(&mut self, command_line: &CommandLine) -> Result<&mut Self, Error> {
        // Set hostname for Ws or Wss protocol
        command_line.hostname.clone().map(|hostname| {
            match &mut self.protocol {
                Some(ProtocolConfig::Ws { host, .. }) => *host = hostname,
                Some(ProtocolConfig::Wss { host, .. }) => *host = hostname,
                _ => {} // just ignore this. or return an error?
            }
        });

        // Set port for Ws or Wss protocol
        command_line.port.map(|new_port| {
            match &mut self.protocol {
                Some(ProtocolConfig::Ws { port, .. }) => *port = new_port,
                Some(ProtocolConfig::Wss { port, .. }) => *port = new_port,
                _ => () // just ignore this. or return an error?
            }
        });

        // Set consensus type
        command_line.consensus_type.map(|consensus| self.consensus(consensus));

        // Set network ID
        command_line.network.map(|network| self.network(network));

        // NOTE: We're always return `Ok(_)`, but we might want to introduce errors later.
        Ok(self)
    }
}
