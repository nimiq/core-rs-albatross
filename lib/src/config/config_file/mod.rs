use std::{
    collections::HashMap, fmt::Debug, fs, io, io::Write as _, num::NonZeroU8, path::Path,
    str::FromStr,
};

use log::level_filters::LevelFilter;
#[cfg(feature = "nimiq-mempool")]
use nimiq_mempool::{
    config::MempoolConfig,
    filter::{MempoolFilter, MempoolRules},
    mempool::Mempool,
};
use nimiq_network_interface::Multiaddr;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::Deserialize;
use nimiq_utils::Sensitive;
use thiserror::Error;
use url::Url;

use crate::{
    config::{command_line::CommandLine, config, config_file::serialization::*, paths},
    error::Error,
};

mod serialization;

// TODO: We have to make more settings `Option`s, so that they can use the `ConfigBuilder`'s
// default and don't overwrite a setting even though it's not set in the config file.

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default)]
    pub network: NetworkSettings,
    #[serde(default)]
    pub consensus: ConsensusSettings,
    #[serde(default)]
    pub zk_prover: Option<ZKProverSettings>,
    pub rpc_server: Option<RpcServerSettings>,
    pub metrics_server: Option<MetricsServerSettings>,
    #[serde(default)]
    pub log: LogSettings,
    #[serde(default)]
    pub prover_log: LogSettings,
    pub database: Option<DatabaseSettings>,
    #[cfg(feature = "nimiq-mempool")]
    pub mempool: Option<MempoolSettings>,
    #[serde(default)]
    pub validator: Option<ValidatorSettings>,
}

impl ConfigFile {
    /// Contents of the default config file as static string
    ///
    /// # ToDo:
    ///
    /// * Change example config file for Albatross
    ///
    const EXAMPLE_CONFIG: &'static str = include_str!("client.example.toml");

    /// Parse config file from file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<ConfigFile, Error> {
        Self::from_str(&fs::read_to_string(path)?)
    }

    fn create_example(path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if parent != Path::new("") {
                if let Err(error) = fs::create_dir_all(parent) {
                    log::warn!(
                        %error,
                        "Failed to create Nimiq home directory {}",
                        parent.display(),
                    );
                }
            }
        }

        log::info!("Creating example config at: {}", path.display());
        let mut file = match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
        {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Ok(());
            }
            result => result?,
        };

        file.write_all(Self::EXAMPLE_CONFIG.as_bytes())
    }

    /// Find config file.
    ///
    /// If the config file location was overwritten by the optional command line argument, it will
    /// try this and possibly fail.
    ///
    /// Otherwise it will look into the default location, which is ~/.nimiq
    ///
    /// # ToDo
    ///
    /// * Add support for environment variable
    ///
    pub fn find(command_line_opt: Option<&CommandLine>) -> Result<ConfigFile, Error> {
        // If the path was set by the command line, only try this path
        if let Some(command_line) = command_line_opt {
            if let Some(path) = &command_line.config {
                return Self::from_file(path);
            }
        }

        // Load config.
        let path = paths::home().join("client.toml");
        match Self::from_file(&path) {
            Err(Error::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
                // Fall through.
            }
            result => return result,
        }

        // The config doesn't exist. Create an example config file and tell
        // the user to create a config.
        let example = paths::home().join("client.toml.example");
        let example_message = Self::create_example(&example)
            .map(|()| format!("see example config file at {}", example.display()))
            .unwrap_or_else(|error| {
                log::warn!(
                    %error,
                    "Failed to create example config file at {}",
                    example.display(),
                );
                format!(
                    "failed to create config file at {}: {}",
                    example.display(),
                    error,
                )
            });

        Err(Error::config_error(format!(
            "Config file not found. Please create one at {} or specify a location using -c path/to/config.toml ({})",
            path.display(),
            example_message,
        )))
    }
}

impl FromStr for ConfigFile {
    type Err = Error;

    /// Parse config file from string
    fn from_str(s: &str) -> Result<ConfigFile, Self::Err> {
        Ok(toml::from_str(s)?)
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkSettings {
    pub peer_key_file: Option<String>,
    pub peer_key: Option<Sensitive<String>>,

    #[serde(default)]
    pub listen_addresses: Vec<String>,

    #[serde(default)]
    pub advertised_addresses: Option<Vec<String>>,

    #[serde(default = "NetworkSettings::default_peer_count_max")]
    pub peer_count_max: usize,
    #[serde(default = "NetworkSettings::peer_count_per_ip_max")]
    pub peer_count_per_ip_max: usize,
    #[serde(default = "NetworkSettings::peer_count_per_ip_max")]
    pub peer_count_per_subnet_max: usize,

    #[serde(default)]
    pub seed_nodes: Vec<Seed>,
    #[serde(default)]
    pub user_agent: Option<String>,

    pub tls: Option<TlsSettings>,
    pub instant_inbound: Option<bool>,
    #[serde(default = "NetworkSettings::default_desired_peer_count")]
    pub desired_peer_count: usize,
    #[serde(default)]
    pub allow_loopback_addresses: bool,
    #[serde(default)]
    pub dht_quorum: Option<NonZeroU8>,
}

impl NetworkSettings {
    pub fn default_desired_peer_count() -> usize {
        12
    }

    pub fn default_peer_count_max() -> usize {
        4000
    }

    pub fn peer_count_per_ip_max() -> usize {
        20
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Seed {
    pub address: Multiaddr,
}

/// Settings for configuring TLS for secure WebSocket
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSettings {
    /// Path to a file containing the private key (PEM-encoded ASN.1 in either PKCS#8 or PKCS#1 format).
    pub private_key: String,
    /// Path to a file containing the certificates (in PEM-encoded X.509 format). In this file several certificates
    /// could be added for certificate chaining.
    pub certificates: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Different knobs used to tweak the consensus mechanism and settings
pub struct ConsensusSettings {
    #[serde(default)]
    /// Possible synchronization modes to reach consensus according to the client type
    pub sync_mode: SyncMode,
    #[serde(default)]
    /// The maximum amount of epochs that are stored in the client
    pub max_epochs_stored: usize,
    /// Different possible networks (MainAlbatross, TestAlbatross, DevAlbatross, UnitAlbatross)
    pub network: Option<NetworkId>,
    /// Minimum number of peers necessary to reach consensus
    pub min_peers: Option<usize>,
    /// Minimum distance away, in number of blocks, from the head to switch from state sync to live sync
    pub full_sync_threshold: Option<u32>,
    /// History indices enabled. Only effective for history nodes (default: `true`)
    #[serde(default = "default_true")]
    pub index_history: bool,
}

impl Default for ConsensusSettings {
    fn default() -> Self {
        Self {
            sync_mode: SyncMode::default(),
            max_epochs_stored: 1,
            network: None,
            min_peers: None,
            full_sync_threshold: None,
            index_history: true,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Debug, Default, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
/// Synchronization mode used by the client based upon its client type
pub enum SyncMode {
    /// Synchronization mode used by History nodes (full transaction history is maintained)
    #[default]
    History,
    /// Full Nodes. They use LightMacroSync + State Sync to reach consensus
    Full,
    /// Light nodes use LightMacroSync + BlockLiveSync to reach consensus
    Light,
}

#[derive(Debug, Error)]
#[error("Invalid sync mode: {0}")]
pub struct SyncModeParseError(String);

impl FromStr for SyncMode {
    type Err = SyncModeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "history" => Self::History,
            "full" => Self::Full,
            "light" => Self::Light,
            _ => return Err(SyncModeParseError(s.to_string())),
        })
    }
}

impl From<SyncMode> for config::SyncMode {
    fn from(sync_mode: SyncMode) -> Self {
        match sync_mode {
            SyncMode::History => Self::History,
            SyncMode::Full => Self::Full,
            SyncMode::Light => Self::Light,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RpcServerSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub bind: Option<String>,
    pub port: Option<u16>,
    #[serde(default)]
    pub corsdomain: Vec<String>,
    #[serde(default)]
    pub allowip: Vec<String>,
    #[serde(default)]
    pub methods: Vec<String>,
    pub username: Option<String>,
    pub password: Option<Sensitive<String>>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricsServerSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub bind: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<Sensitive<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LokiConfig {
    pub url: Url,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub extra_fields: HashMap<String, String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub level: Option<LevelFilter>,
    #[serde(default = "default_true")]
    pub timestamps: bool,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_tags")]
    pub tags: HashMap<String, LevelFilter>,
    #[serde(default = "LogSettings::default_statistics_interval")]
    pub statistics: u64,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub loki: Option<LokiConfig>,
    #[serde(default)]
    pub tokio_console_bind_address: Option<String>,
}

impl LogSettings {
    pub fn default_statistics_interval() -> u64 {
        10
    }
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            level: None,
            timestamps: true,
            tags: HashMap::new(),
            statistics: 10,
            file: None,
            loki: None,
            tokio_console_bind_address: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseSettings {
    pub path: Option<String>,
    pub size: Option<usize>,
    pub max_dbs: Option<u32>,
    pub max_readers: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MempoolSettings {
    pub filter: Option<MempoolFilterSettings>,
    pub size_limit: Option<usize>,
    pub control_size_limit: Option<usize>,
    pub blacklist_limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MempoolFilterSettings {
    /// Minimum fee for all transactions
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub tx_fee: Coin,
    /// Minimum fee per byte for all transactions
    #[serde(default)]
    pub tx_fee_per_byte: f64,
    /// Minimum value for all transactions
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub tx_value: Coin,
    /// Minimum total value (value + fee) for all transactions
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub tx_value_total: Coin,
    /// Minimum fee for transactions creating a contract
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub contract_fee: Coin,
    /// Minimum fee per byte for transactions creating a contract
    #[serde(default)]
    pub contract_fee_per_byte: f64,
    /// Minimum value for transactions creating a contract
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub contract_value: Coin,
    /// Minimum fee for transactions creating a new account
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub creation_fee: Coin,
    /// Minimum fee per byte for transactions creating a new account
    #[serde(default)]
    pub creation_fee_per_byte: f64,
    /// Minimum value for transactions creating a new account
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub creation_value: Coin,
    /// Minimum balance that the recipient account must have after the transaction
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub recipient_balance: Coin,
    /// Minimum balance that must remain on the sender account after the transaction, if not zero
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub sender_balance: Coin,
}

#[cfg(feature = "nimiq-mempool")]
impl From<MempoolSettings> for MempoolConfig {
    fn from(mempool: MempoolSettings) -> Self {
        Self {
            size_limit: mempool.size_limit.unwrap_or(Mempool::DEFAULT_SIZE_LIMIT),
            control_size_limit: mempool
                .control_size_limit
                .unwrap_or(Mempool::DEFAULT_CONTROL_SIZE_LIMIT),
            filter_limit: mempool
                .blacklist_limit
                .unwrap_or(MempoolFilter::DEFAULT_BLACKLIST_SIZE),
            filter_rules: mempool.filter.map(MempoolRules::from).unwrap_or_default(),
        }
    }
}

/// Convert mempool settings
#[cfg(feature = "nimiq-mempool")]
impl From<MempoolFilterSettings> for MempoolRules {
    fn from(f: MempoolFilterSettings) -> MempoolRules {
        Self {
            tx_fee: f.tx_fee,
            tx_fee_per_byte: f.tx_fee_per_byte,
            tx_value: f.tx_value,
            tx_value_total: f.tx_value_total,
            contract_fee: f.contract_fee,
            contract_fee_per_byte: f.contract_fee_per_byte,
            contract_value: f.contract_value,
            creation_fee: f.creation_fee,
            creation_fee_per_byte: f.creation_fee_per_byte,
            creation_value: f.creation_value,
            sender_balance: f.sender_balance,
            recipient_balance: f.recipient_balance,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ValidatorSettings {
    pub validator_address: String,
    pub signing_key_file: Option<String>,
    pub signing_key: Option<Sensitive<String>>,
    pub voting_key_file: Option<String>,
    pub voting_key: Option<Sensitive<String>>,
    pub fee_key_file: Option<String>,
    pub fee_key: Option<Sensitive<String>>,
    #[serde(default)]
    pub automatic_reactivate: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ZKProverSettings {
    #[serde(default)]
    pub prover_keys_path: Option<String>,
}
