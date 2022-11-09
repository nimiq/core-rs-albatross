use std::collections::HashMap;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use log::level_filters::LevelFilter;
use serde_derive::Deserialize;
use thiserror::Error;
use url::Url;

use nimiq_mempool::mempool::Mempool;
use nimiq_mempool::{
    config::MempoolConfig,
    filter::{MempoolFilter, MempoolRules},
};
use nimiq_network_libp2p::Multiaddr;
use nimiq_primitives::{coin::Coin, networks::NetworkId};

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
    pub zkp: Option<ZKPSettings>,
    pub rpc_server: Option<RpcServerSettings>,
    pub metrics_server: Option<MetricsServerSettings>,
    #[serde(default)]
    pub log: LogSettings,
    pub database: Option<DatabaseSettings>,
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
        Self::from_str(&read_to_string(path)?)
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

        // If example doesn't exist, create it.
        let path_example = paths::home().join("client.toml.example");
        if !path_example.exists() {
            log::info!("Creating example config at: {}", path_example.display());
            if let Err(e) = std::fs::write(&path_example, Self::EXAMPLE_CONFIG) {
                log::warn!(
                    "Failed to create example config file: {}: {}",
                    e,
                    path_example.display()
                );
            }
        }

        // Check if config exists, otherwise tell user to create one.
        let path = paths::home().join("client.toml");
        if !path.exists() {
            let msg = format!(
                "Config file not found. Please create one at {} or specify a location using -c path/to/config.toml, see the example config file at {}",
                path.display(),
                path_example.display(),
            );
            log::warn!("{}", msg);
            return Err(Error::config_error(&msg));
        }

        // Load config.
        Self::from_file(&path)
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
    pub peer_key: Option<String>,

    #[serde(default)]
    pub listen_addresses: Vec<String>,

    #[serde(default)]
    pub seed_nodes: Vec<Seed>,
    #[serde(default)]
    pub user_agent: Option<String>,

    pub tls: Option<TlsSettings>,
    pub instant_inbound: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Seed {
    pub address: Multiaddr,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSettings {
    pub identity_file: String,
    pub identity_password: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConsensusSettings {
    #[serde(default)]
    pub sync_mode: SyncMode,
    #[serde(default)]
    pub max_epochs_stored: usize,
    #[serde(default)]
    pub network: Network,
    pub min_peers: Option<usize>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    History,
    Full,
    Nano,
}
impl Default for SyncMode {
    fn default() -> Self {
        SyncMode::History
    }
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
            "nano" => Self::Nano,
            _ => return Err(SyncModeParseError(s.to_string())),
        })
    }
}

impl From<SyncMode> for config::SyncMode {
    fn from(sync_mode: SyncMode) -> Self {
        match sync_mode {
            SyncMode::History => Self::History,
            SyncMode::Full => Self::Full,
            SyncMode::Nano => Self::Nano,
        }
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
// TODO: I think we can directly use `NetworkId` here
pub enum Network {
    Main,
    Test,
    Dev,
    TestAlbatross,
    DevAlbatross,
    UnitAlbatross,
}

impl Default for Network {
    fn default() -> Self {
        Network::DevAlbatross
    }
}

impl FromStr for Network {
    type Err = ();

    fn from_str(s: &str) -> Result<Network, ()> {
        Ok(match s.to_lowercase().as_str() {
            "main" => Network::Main,
            "test" => Network::Test,
            "dev" => Network::Dev,
            "test-albatross" => Network::TestAlbatross,
            "dev-albatross" => Network::DevAlbatross,
            "unit-albatross" => Network::UnitAlbatross,
            _ => return Err(()),
        })
    }
}

impl From<Network> for NetworkId {
    fn from(network: Network) -> NetworkId {
        match network {
            Network::Main => NetworkId::Main,
            Network::Test => NetworkId::Test,
            Network::Dev => NetworkId::Dev,
            Network::TestAlbatross => NetworkId::TestAlbatross,
            Network::DevAlbatross => NetworkId::DevAlbatross,
            Network::UnitAlbatross => NetworkId::UnitAlbatross,
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
    pub password: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricsServerSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub bind: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotatingLogFileConfig {
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub size: usize,
    #[serde(default)]
    pub file_count: usize,
}

impl Default for RotatingLogFileConfig {
    fn default() -> Self {
        Self {
            path: paths::home().join("logs"),
            size: 50_000_000, // 50 MB
            file_count: 3,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub level: Option<LevelFilter>,
    #[serde(default)]
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
    pub rotating_trace_log: Option<RotatingLogFileConfig>,
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
            rotating_trace_log: None,
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
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub tx_fee: Coin,
    #[serde(default)]
    pub tx_fee_per_byte: f64,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub tx_value: Coin,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub tx_value_total: Coin,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub contract_fee: Coin,
    #[serde(default)]
    pub contract_fee_per_byte: f64,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub contract_value: Coin,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_coin")]
    pub creation_fee: Coin,
    #[serde(default)]
    pub creation_fee_per_byte: f64,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub creation_value: Coin,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub recipient_balance: Coin,
    #[serde(deserialize_with = "deserialize_coin")]
    #[serde(default)]
    pub sender_balance: Coin,
}

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
    pub signing_key: Option<String>,
    pub voting_key_file: Option<String>,
    pub voting_key: Option<String>,
    pub fee_key_file: Option<String>,
    pub fee_key: Option<String>,
    #[serde(default)]
    pub automatic_reactivate: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ZKPSettings {
    #[serde(default)]
    pub prover_active: bool,
    #[serde(default)]
    pub setup_keys_path: Option<String>,
    #[serde(default)]
    pub zkp_propagation: bool,
}
