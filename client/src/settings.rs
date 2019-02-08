use std::path::Path;
use std::fs::read_to_string;
use std::error::Error;
use std::collections::HashMap;


pub const DEFAULT_NETWORK_PORT: u16 = 8443;
pub const DEFAULT_REVERSE_PROXY_PORT: u16 = 8444;
pub const DEFAULT_RPC_PORT: u16 = 8648;
pub const DEFAULT_METRICS_PORT: u16 = 8649;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settings {
    pub network: NetworkSettings,
    pub tls: Option<TlsSettings>,
    #[serde(default)]
    pub consensus: ConsensusSettings,
    pub miner: Option<MinerSettings>,
    pub pool_mining: Option<PoolMiningSettings>,
    pub rpc_server: Option<RpcServerSettings>,
    pub ui_server: Option<UiServerSettings>,
    pub metrics_server: Option<MetricsServerSettings>,
    pub wallet: Option<WalletSettings>,
    pub reverse_proxy: Option<ReverseProxySettings>,
    #[serde(default)]
    pub log: LogSettings,
    #[serde(default)]
    pub database: DatabaseSettings
}

impl Settings {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Settings, Box<dyn Error>> {
        Ok(toml::from_str(read_to_string(path)?.as_ref())?)
    }
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkSettings {
    pub host: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default)]
    pub passive: bool,
    #[serde(default)]
    pub seed_nodes: Vec<SeedNode>,
    #[serde(default)]
    pub user_agent: Option<String>
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
        Protocol::Dumb
    }
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedNode {
    pub host: String,
    pub port: u16,
    pub public_key: Option<String>,
}



#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TlsSettings {
    pub cert: String,
    pub key: String,
}


#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
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

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Network {
    Main,
    Test,
    Dev,
    Dummy,
    Bounty
}

impl Default for Network {
    fn default() -> Self {
        Network::Main
    }
}



#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinerSettings {
    pub threads: Option<u64>,
    pub throttle_after: Option<u64>, // TODO: Fix that this can take "infinity"
    pub throttle_wait: Option<u64>,
    pub extra_data: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct PoolMiningSettings {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub mode: MiningPoolMode,
    // TODO: This only accepts String -> String.
    #[serde(default)]
    pub device_data: HashMap<String, String>
}


#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MiningPoolMode {
    Smart,
    Nano,
}

impl Default for MiningPoolMode {
    fn default() -> Self {
        MiningPoolMode::Smart
    }
}


#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RpcServerSettings {
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

#[derive(Debug, Deserialize, Default)]
pub(crate) struct UiServerSettings {
    pub port: u16,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct MetricsServerSettings {
    pub port: Option<u16>,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct WalletSettings {
    pub seed: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ReverseProxySettings {
    pub port: Option<u16>,
    pub address: String,
    pub header: String,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct LogSettings {
    pub level: Option<String>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default)]
    pub statistics: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DatabaseSettings {
    pub path: String,
    pub size: usize,
    pub max_dbs: u32
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        DatabaseSettings {
            path: String::from("./db/"),
            size: 1024 * 1024 * 50,
            max_dbs: 10
        }
    }
}