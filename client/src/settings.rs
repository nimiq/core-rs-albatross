use std::collections::HashMap;
use std::fs::read_to_string;
use std::path::Path;
use std::str::FromStr;

use failure::Error;
use log::LevelFilter;

use network::network_config::Seed;
use network_primitives::address::NetAddress;
use primitives::coin::Coin;

use crate::serialization::*;

pub const DEFAULT_REVERSE_PROXY_PORT: u16 = 8444;
pub const DEFAULT_RPC_PORT: u16 = 8648;
pub const DEFAULT_METRICS_PORT: u16 = 8649;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(default)]
    pub network: NetworkSettings,
    #[serde(default)]
    pub consensus: ConsensusSettings,
    pub rpc_server: Option<RpcServerSettings>,
    pub metrics_server: Option<MetricsServerSettings>,
    //pub wallet: Option<WalletSettings>,
    pub reverse_proxy: Option<ReverseProxySettings>,
    #[serde(default)]
    pub log: LogSettings,
    #[serde(default)]
    pub database: DatabaseSettings,
    pub mempool: Option<MempoolSettings>,
}

impl Settings {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Settings, Error> {
        Ok(toml::from_str(read_to_string(path)?.as_ref())?)
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct NetworkSettings {
    pub host: Option<String>,
    pub port: Option<u16>,
    #[serde(default)]
    pub protocol: Protocol,
    //#[serde(default)]
    //pub passive: bool,
    #[serde(deserialize_with = "deserialize_string_vec")]
    #[serde(default)]
    pub seed_nodes: Vec<Seed>,
    #[serde(default)]
    pub user_agent: Option<String>,
    pub tls: Option<TlsSettings>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Protocol {
    Wss,
    Ws,
    Dumb,
    Rtc,
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::Ws
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TlsSettings {
    pub identity_file: String,
    pub identity_password: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConsensusSettings {
    #[serde(rename = "type")]
    #[serde(default)]
    pub node_type: NodeType,
    #[serde(default)]
    pub network: Network,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NodeType {
    Full,
    Light,
    Nano,
}

impl Default for NodeType {
    fn default() -> Self {
        NodeType::Full
    }
}

impl FromStr for NodeType {
    type Err = ();

    fn from_str(s: &str) -> Result<NodeType, ()> {
        Ok(match s.to_lowercase().as_str() {
            "full" => NodeType::Full,
            "light" => NodeType::Light,
            "nano" => NodeType::Nano,
            _ => Err(())?
        })
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Network {
    Main,
    Test,
    Dev,
}

impl Default for Network {
    fn default() -> Self {
        Network::Main
    }
}

impl FromStr for Network {
    type Err = ();

    fn from_str(s: &str) -> Result<Network, ()> {
        Ok(match s.to_lowercase().as_str() {
            "main" => Network::Main,
            "test" => Network::Test,
            "dev" => Network::Dev,
            _ => Err(())?
        })
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RpcServerSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub bind: Option<NetAddress>,
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

/*#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct UiServerSettings {
    pub port: u16,
}*/

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricsServerSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    #[serde(default)]
    pub bind: Option<NetAddress>,
    pub port: Option<u16>,
    pub password: Option<String>,
}

/*#[derive(Debug, Deserialize, Default)]
pub(crate) struct WalletSettings {
    pub seed: Option<String>,
    pub address: Option<String>,
}*/

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReverseProxySettings {
    pub port: Option<u16>,
    #[serde(deserialize_with = "deserialize_string")]
    pub address: NetAddress,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub with_tls_termination: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogSettings {
    #[serde(deserialize_with = "deserialize_string_option")]
    pub level: Option<LevelFilter>,
    #[serde(default)]
    pub timestamps: bool,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_tags")]
    pub tags: HashMap<String, LevelFilter>,
    #[serde(default)]
    pub statistics: u64,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DatabaseSettings {
    pub path: String,
    pub size: Option<usize>,
    pub max_dbs: Option<u32>
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        DatabaseSettings {
            path: String::from("./db/"),
            size: Some(1024 * 1024 * 50),
            max_dbs: Some(10)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MempoolSettings {
    pub filter: Option<MempoolFilterSettings>,
    pub blacklist_limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MempoolFilterSettings {
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
