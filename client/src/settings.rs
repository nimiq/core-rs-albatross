use std::path::Path;
use std::fs::read_to_string;
use failure::Error;
use std::collections::HashMap;
use std::str::FromStr;

pub const DEFAULT_NETWORK_PORT: u16 = 8443;
pub const DEFAULT_REVERSE_PROXY_PORT: u16 = 8444;
pub const DEFAULT_RPC_PORT: u16 = 8648;
pub const DEFAULT_METRICS_PORT: u16 = 8649;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Settings {
    #[serde(default)]
    pub network: NetworkSettings,
    pub tls: Option<TlsSettings>, // TODO: Move this into `NetworkSettings`?
    #[serde(default)]
    pub consensus: ConsensusSettings,
    pub rpc_server: Option<RpcServerSettings>,
    pub metrics_server: Option<MetricsServerSettings>,
    //pub wallet: Option<WalletSettings>,
    pub reverse_proxy: Option<ReverseProxySettings>,
    #[serde(default)]
    pub log: LogSettings,
    #[serde(default)]
    pub database: DatabaseSettings
}

impl Settings {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Settings, Error> {
        Ok(toml::from_str(read_to_string(path)?.as_ref())?)
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct NetworkSettings {
    pub host: Option<String>,
    pub port: Option<u16>,
    #[serde(default)]
    pub protocol: Protocol,
    //#[serde(default)]
    //pub passive: bool,
    #[serde(default)]
    pub seed_nodes: Vec<String>,
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
pub(crate) struct TlsSettings {
    pub identity_file: String,
    pub identity_password: String,
}

#[derive(Debug, Deserialize, Default)]
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
    type Err = (); // TODO: proper Error type

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
    type Err = (); // TODO: proper Error type

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

/*#[derive(Debug, Deserialize, Default)]
pub(crate) struct WalletSettings {
    pub seed: Option<String>,
    pub address: Option<String>,
}*/

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ReverseProxySettings {
    pub port: Option<u16>,
    pub address: String,
    pub header: String,
    #[serde(default)]
    pub with_tls_termination: bool,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct LogSettings {
    pub level: Option<String>,
    #[serde(default)]
    pub timestamps: bool,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default)]
    pub statistics: u64,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize)]
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
