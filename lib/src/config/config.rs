#[cfg(feature = "rpc-server")]
use std::net::IpAddr;
#[cfg(feature = "metrics-server")]
use std::net::SocketAddr;
use std::{
    path::{Path, PathBuf},
    string::ToString,
};

use derive_builder::Builder;
use strum_macros::Display;

use beserial::Deserialize;
#[cfg(feature = "validator")]
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_database::{mdbx::MdbxEnvironment, volatile::VolatileEnvironment, Environment};
#[cfg(feature = "validator")]
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::{config::MempoolConfig, filter::MempoolRules};
use nimiq_network_libp2p::{Keypair as IdentityKeypair, Multiaddr};
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::file_store::FileStore;
#[cfg(feature = "validator")]
use nimiq_utils::key_rng::SecureGenerate;

#[cfg(any(feature = "rpc-server", feature = "metrics-server"))]
use crate::config::consts;
#[cfg(feature = "metrics-server")]
use crate::config::consts::default_bind;
use crate::{
    client::Client,
    config::{
        command_line::CommandLine,
        config_file::{self, ConfigFile, Seed},
        paths,
        user_agent::UserAgent,
    },
    error::Error,
};

/// The sync mode
///
/// # Notes
///
/// core-rs / Albatross currently only supports history sync.
///
/// # ToDo
///
/// * We'll propably have this enum somewhere in the primitives. So this is a placeholder.
///
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Display)]
pub enum SyncMode {
    History,
}

impl Default for SyncMode {
    fn default() -> Self {
        Self::History
    }
}

#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct ConsensusConfig {
    #[builder(default)]
    pub sync_mode: SyncMode,
    #[builder(default = "3")]
    pub min_peers: usize,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        ConsensusConfig {
            sync_mode: SyncMode::default(),
            min_peers: 3,
        }
    }
}

/// Network config
#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct NetworkConfig {
    #[builder(default)]
    pub listen_addresses: Vec<Multiaddr>,

    /// The user agent is a custom string that is sent during the handshake. Usually it contains
    /// the kind of node, Nimiq version, processor architecture and operating system. This enable
    /// gathering information on which Nimiq versions are being run on the network. A typical
    /// user agent would be `core-rs-albatross/0.1.0 (native; linux x86_64)`
    ///
    /// Default will generate a value from system information. This is recommended.
    ///
    #[builder(default)]
    pub user_agent: UserAgent,

    #[builder(default)]
    pub seeds: Vec<Seed>,
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

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct FileStorageConfig {
    /// The parent directory where the database will be stored. The database directory name
    /// is determined by the network ID and consensus type using the `database_name` static
    /// method.
    pub database_parent: PathBuf,

    /// Path to peer key.
    pub peer_key_path: PathBuf,

    /// The key used for the peer key, if the file is not present.
    pub peer_key: Option<String>,

    /// Path to voting key.
    #[cfg(feature = "validator")]
    pub voting_key_path: Option<PathBuf>,

    /// The voting key used for the validator, if the file is not present.
    #[cfg(feature = "validator")]
    pub voting_key: Option<String>,

    /// Path to signing key.
    #[cfg(feature = "validator")]
    pub signing_key_path: Option<PathBuf>,

    /// The signing key used for the validator, if the file is not present.
    #[cfg(feature = "validator")]
    pub signing_key: Option<String>,

    /// Path to fee key.
    #[cfg(feature = "validator")]
    pub fee_key_path: Option<PathBuf>,

    /// The fee key used for the validator, if the file is not present.
    #[cfg(feature = "validator")]
    pub fee_key: Option<String>,
}

impl FileStorageConfig {
    /// Create storage config from a directory path.
    ///
    pub fn from_directory<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        Self {
            database_parent: path.to_path_buf(),
            peer_key_path: path.join("peer_key.dat"),
            peer_key: None,
            #[cfg(feature = "validator")]
            voting_key_path: Some(path.join("voting_key.dat")),
            #[cfg(feature = "validator")]
            voting_key: None,
            #[cfg(feature = "validator")]
            fee_key_path: Some(path.join("fee_key.dat")),
            #[cfg(feature = "validator")]
            fee_key: None,
            #[cfg(feature = "validator")]
            signing_key_path: Some(path.join("signing_key.dat")),
            #[cfg(feature = "validator")]
            signing_key: None,
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
#[derive(Debug, Clone, Builder, Eq, PartialEq)]
#[builder(setter(into))]
pub struct DatabaseConfig {
    /// Initial database size. Default: 1 TB
    #[builder(default = "1024 * 1024 * 1024 * 1024")]
    size: usize,

    /// Max number of DBs. Recommended: 12
    #[builder(default = "12")]
    max_dbs: u32,

    /// Max number of threads that can open read transactions.
    /// Tokio by default has a maximum of 1 + num cores + 512 (blocking) threads.
    /// Our default value allows for up to 87 cores if tokio's defaults are not changed.
    /// Recommended: 600
    #[builder(default = "600")]
    max_readers: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            // 1 TB
            size: 1024 * 1024 * 1024 * 1024,
            max_dbs: 12,
            max_readers: 600,
        }
    }
}

impl From<Option<config_file::DatabaseSettings>> for DatabaseConfig {
    fn from(db_settings: Option<config_file::DatabaseSettings>) -> Self {
        let default = DatabaseConfig::default();

        if let Some(db_settings) = db_settings {
            Self {
                size: db_settings.size.unwrap_or(default.size),
                max_dbs: db_settings.max_dbs.unwrap_or(default.max_dbs),
                max_readers: db_settings.max_readers.unwrap_or(default.max_readers),
            }
        } else {
            default
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
    pub fn database(
        &self,
        network_id: NetworkId,
        sync_mode: SyncMode,
        db_config: DatabaseConfig,
    ) -> Result<Environment, Error> {
        let db_name = format!("{}-{}-consensus", network_id, sync_mode).to_lowercase();
        log::info!("Opening database: {}", db_name);

        Ok(match self {
            StorageConfig::Volatile => {
                VolatileEnvironment::with_max_readers(db_config.max_dbs, db_config.max_readers)?
            }
            StorageConfig::Filesystem(file_storage) => {
                let db_path = file_storage.database_parent.join(db_name);
                let db_path = db_path
                    .to_str()
                    .ok_or_else(|| {
                        Error::config_error(format!(
                            "Failed to convert database path to string: {}",
                            db_path.display()
                        ))
                    })?
                    .to_string();
                MdbxEnvironment::new_with_max_readers(
                    &db_path,
                    db_config.size,
                    db_config.max_dbs,
                    db_config.max_readers,
                )?
            }
            _ => return Err(self.not_available()),
        })
    }

    #[cfg(feature = "validator")]
    pub(crate) fn voting_keypair(&self) -> Result<BlsKeyPair, Error> {
        Ok(match self {
            StorageConfig::Volatile => BlsKeyPair::generate_default_csprng(),
            StorageConfig::Filesystem(file_storage) => {
                let key_path = file_storage
                    .voting_key_path
                    .as_ref()
                    .ok_or_else(|| Error::config_error("No path for validator key specified"))?;
                let key_path = key_path
                    .to_str()
                    .ok_or_else(|| {
                        Error::config_error(format!(
                            "Failed to convert path of validator key to string: {}",
                            key_path.display()
                        ))
                    })?
                    .to_string();

                FileStore::new(key_path).load_or_store(|| {
                    if let Some(key) = file_storage.voting_key.as_ref() {
                        // TODO: handle errors
                        let secret_key =
                            BlsSecretKey::deserialize_from_vec(&hex::decode(key).unwrap()).unwrap();
                        secret_key.into()
                    } else {
                        BlsKeyPair::generate_default_csprng()
                    }
                })?
            }
            _ => return Err(self.not_available()),
        })
    }

    #[cfg(feature = "validator")]
    pub(crate) fn fee_keypair(&self) -> Result<KeyPair, Error> {
        Ok(match self {
            StorageConfig::Volatile => KeyPair::generate_default_csprng(),
            StorageConfig::Filesystem(file_storage) => {
                let key_path = file_storage
                    .fee_key_path
                    .as_ref()
                    .ok_or_else(|| Error::config_error("No path for fee key specified"))?;
                let key_path = key_path
                    .to_str()
                    .ok_or_else(|| {
                        Error::config_error(format!(
                            "Failed to convert path of fee key to string: {}",
                            key_path.display()
                        ))
                    })?
                    .to_string();

                FileStore::new(key_path).load_or_store(|| {
                    if let Some(key) = file_storage.fee_key.as_ref() {
                        // TODO: handle errors
                        KeyPair::from(
                            PrivateKey::deserialize_from_vec(&hex::decode(key).unwrap()).unwrap(),
                        )
                    } else {
                        KeyPair::generate_default_csprng()
                    }
                })?
            }
            _ => return Err(self.not_available()),
        })
    }

    #[cfg(feature = "validator")]
    pub(crate) fn signing_keypair(&self) -> Result<KeyPair, Error> {
        Ok(match self {
            StorageConfig::Volatile => KeyPair::generate_default_csprng(),
            StorageConfig::Filesystem(file_storage) => {
                let key_path = file_storage
                    .signing_key_path
                    .as_ref()
                    .ok_or_else(|| Error::config_error("No path for warm key specified"))?;
                let key_path = key_path
                    .to_str()
                    .ok_or_else(|| {
                        Error::config_error(format!(
                            "Failed to convert path of warm key to string: {}",
                            key_path.display()
                        ))
                    })?
                    .to_string();

                FileStore::new(key_path).load_or_store(|| {
                    if let Some(key) = file_storage.signing_key.as_ref() {
                        // TODO: handle errors
                        KeyPair::from(
                            PrivateKey::deserialize_from_vec(&hex::decode(key).unwrap()).unwrap(),
                        )
                    } else {
                        KeyPair::generate_default_csprng()
                    }
                })?
            }
            _ => return Err(self.not_available()),
        })
    }

    pub(crate) fn identity_keypair(&self) -> Result<IdentityKeypair, Error> {
        match self {
            StorageConfig::Volatile => Ok(IdentityKeypair::generate_ed25519()),
            StorageConfig::Filesystem(file_storage) => {
                Ok(
                    FileStore::new(&file_storage.peer_key_path).load_or_store(|| {
                        if let Some(key) = file_storage.peer_key.as_ref() {
                            // TODO: handle errors
                            IdentityKeypair::deserialize_from_vec(&hex::decode(key).unwrap())
                                .unwrap()
                        } else {
                            IdentityKeypair::generate_ed25519()
                        }
                    })?,
                )
            }
            _ => Err(self.not_available()),
        }
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

#[cfg(feature = "validator")]
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ValidatorConfig {
    /// The validator address.
    pub validator_address: Address,

    /// Config if the validator automatically reactivates itself.
    pub automatic_reactivate: bool,
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

#[cfg(feature = "rpc-server")]
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
    #[builder(default = "consts::RPC_DEFAULT_PORT")]
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

#[cfg(feature = "metrics-server")]
#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct MetricsServerConfig {
    /// Bind the server to the specified ip and port.
    ///
    /// Default: `127.0.0.1:9100`
    ///
    pub addr: SocketAddr,

    /// If specified, require HTTP basic auth with these credentials
    #[builder(setter(strip_option))]
    pub credentials: Option<Credentials>,
}

/// Client configuration
///
/// # ToDo
///
/// * Make this implement `IntoFuture<Item=Client, Err=Error>` so you can just do
///   `tokio::spawn(config.and_then(|client| [...]));`
#[derive(Clone, Debug, Builder)]
#[builder(setter(into), build_fn(private, name = "build_internal"))]
pub struct ClientConfig {
    /// Network config
    pub network: NetworkConfig,

    /// Consensus config
    #[builder(default)]
    pub consensus: ConsensusConfig,

    /// The Nimiq network the client should connect to. Usually this should be either `Test` or
    /// `Main` for the Nimiq 1.0 networks. For Albatross there is currently only `TestAlbatross`
    /// and `DevAlbatross` available. Since Albatross is still in development at time of writing,
    /// it is recommended to use `DevAlbatross`.
    ///
    /// Default is `DevAlbatross`
    ///
    /// # TODO
    ///
    ///  - Rename, to avoid confusion with the libp2p network
    #[builder(default = "NetworkId::DevAlbatross")]
    pub network_id: NetworkId,

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

    /// The optional validator configuration
    ///
    #[cfg(feature = "validator")]
    #[builder(default)]
    pub validator: Option<ValidatorConfig>,

    /// The optional rpc-server configuration
    ///
    #[cfg(feature = "rpc-server")]
    #[builder(default)]
    pub rpc_server: Option<RpcServerConfig>,

    #[cfg(feature = "metrics-server")]
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
    pub async fn instantiate_client(self) -> Result<Client, Error> {
        Client::from_config(self).await
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
            .map_err(|e| Error::config_error(e.to_string()))
    }

    /// Short cut to build the config and instantiate the client
    ///
    pub async fn instantiate_client(&self) -> Result<Client, Error> {
        self.build()?.instantiate_client().await
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

    /// Configures the storage to be volatile. All data will be lost after shutdown of the client.
    pub fn volatile(&mut self) -> &mut Self {
        self.storage = Some(StorageConfig::Volatile);
        self
    }

    /// Sets the mempool filter rules
    pub fn mempool(
        &mut self,
        size_limit: usize,
        control_size_limit: usize,
        filter_rules: MempoolRules,
        filter_limit: usize,
    ) -> &mut Self {
        self.mempool = Some(MempoolConfig {
            size_limit,
            control_size_limit,
            filter_rules,
            filter_limit,
        });
        self
    }

    /// Applies settings from a configuration file
    pub fn config_file(&mut self, config_file: &ConfigFile) -> Result<&mut Self, Error> {
        // TODO: if the config field of `listen_addresses` is empty, we should at least add `/ip4/127.0.0.1/...`
        self.network(NetworkConfig {
            listen_addresses: config_file
                .network
                .listen_addresses
                .iter()
                .map(|addr| addr.parse())
                .collect::<Result<Vec<Multiaddr>, _>>()?,

            user_agent: config_file
                .network
                .user_agent
                .as_ref()
                .map(|ua| UserAgent::from(ua.to_owned()))
                .unwrap_or_default(),

            seeds: config_file.network.seed_nodes.clone(),
        });

        // Configure consensus
        let mut consensus = ConsensusConfigBuilder::default()
            .sync_mode(config_file.consensus.sync_mode)
            .build()
            .unwrap();
        if let Some(min_peers) = config_file.consensus.min_peers {
            consensus.min_peers = min_peers;
        }
        self.consensus(consensus);

        // Configure network
        self.network_id(config_file.consensus.network);

        // Configure storage config.
        let mut file_storage = FileStorageConfig::default();
        if let Some(db_config_file) = &config_file.database {
            if let Some(path) = db_config_file.path.as_ref() {
                file_storage.database_parent = PathBuf::from(path);
            }
        }
        if let Some(key_path) = config_file.network.peer_key_file.as_ref() {
            file_storage.peer_key_path = PathBuf::from(key_path);
        }
        if let Some(key) = config_file.network.peer_key.as_ref() {
            file_storage.peer_key = Some(key.to_owned());
        }
        #[cfg(feature = "validator")]
        if let Some(validator_config) = config_file.validator.as_ref() {
            self.validator(ValidatorConfig {
                validator_address: Address::from_any_str(&validator_config.validator_address)?,
                automatic_reactivate: validator_config.automatic_reactivate,
            });

            if let Some(key_path) = &validator_config.voting_key_file {
                file_storage.voting_key_path = Some(PathBuf::from(key_path));
            }
            if let Some(key) = &validator_config.voting_key {
                file_storage.voting_key = Some(key.to_owned());
            }
            if let Some(key_path) = &validator_config.fee_key_file {
                file_storage.fee_key_path = Some(PathBuf::from(key_path));
            }
            if let Some(key) = &validator_config.fee_key {
                file_storage.fee_key = Some(key.to_owned());
            }
            if let Some(key_path) = &validator_config.signing_key_file {
                file_storage.signing_key_path = Some(PathBuf::from(key_path));
            }
            if let Some(key) = &validator_config.signing_key {
                file_storage.signing_key = Some(key.to_owned());
            }
        }
        self.storage = Some(file_storage.into());

        // Configure database
        self.database(config_file.database.clone());

        // Configure RPC server
        #[cfg(feature = "rpc-server")]
        {
            if let Some(rpc_config) = &config_file.rpc_server {
                let bind_to = rpc_config
                    .bind
                    .as_ref()
                    .and_then(|addr| addr.into_ip_address());

                let allow_ips = if rpc_config.allowip.is_empty() {
                    None
                } else {
                    let result = rpc_config
                        .allowip
                        .iter()
                        .map(|s| {
                            s.parse::<IpAddr>()
                                .map_err(|e| Error::config_error(format!("Invalid IP: {}", e)))
                        })
                        .collect::<Result<Vec<IpAddr>, Error>>();
                    Some(result?)
                };

                let credentials = match (&rpc_config.username, &rpc_config.password) {
                    (Some(u), Some(p)) => Some(Credentials::new(u.clone(), p.clone())),
                    (None, None) => None,
                    _ => {
                        return Err(Error::config_error(
                            "RTP: Either both username and password have to be set or none.",
                        ))
                    }
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

        // Configure metrics server
        #[cfg(feature = "metrics-server")]
        {
            if let Some(metrics_config) = &config_file.metrics_server {
                let ip = metrics_config
                    .bind
                    .as_ref()
                    .and_then(|addr| addr.into_ip_address());

                let addr = SocketAddr::new(
                    ip.unwrap_or_else(default_bind),
                    metrics_config.port.unwrap_or(consts::METRICS_DEFAULT_PORT),
                );

                let credentials =
                    match (&metrics_config.username, &metrics_config.password) {
                        (Some(u), Some(p)) => Some(Credentials::new(u.clone(), p.clone())),
                        (None, None) => None,
                        _ => return Err(Error::config_error(
                            "Metrics: Either both username and password have to be set or none.",
                        )),
                    };

                self.metrics_server = Some(Some(MetricsServerConfig { addr, credentials }));
            }
        }

        Ok(self)
    }

    /// Applies settings from the command line
    pub fn command_line(&mut self, command_line: &CommandLine) -> Result<&mut Self, Error> {
        // Set sync_mode
        if let Some(sync_mode) = command_line.sync_mode {
            self.consensus
                .get_or_insert_with(ConsensusConfig::default)
                .sync_mode = sync_mode.into()
        }

        // Set network ID
        if let Some(network_id) = command_line.network {
            self.network_id(network_id);
        }

        // NOTE: We're always return `Ok(_)`, but we might want to introduce errors later.
        Ok(self)
    }
}
