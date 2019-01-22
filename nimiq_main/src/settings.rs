use std::path::Path;
use std::fs::read_to_string;
use std::error::Error;
use std::collections::HashMap;


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    network: NetworkSettings,
    tls: Option<TlsSettings>,
    consensus: Option<ConsensusSettings>,
    miner: Option<MinerSettings>,
    pool_mining: Option<PoolMiningSettings>,
    rpc_server: Option<RpcServerSettings>,
    ui_server: Option<UiServerSettings>,
    metrics_server: Option<MetricsServerSettings>,
    wallet: Option<WalletSettings>,
    reverse_proxy: Option<ReverseProxySettings>,
    log: Option<LogSettings>,
}

impl Settings {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Settings, Box<dyn Error>> {
        Ok(toml::from_str(read_to_string(path)?.as_ref())?)
    }
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSettings {
    host: String,
    port: Option<u16>,
    protocol: Option<Protocol>,
    passive: Option<bool>,
    seed_nodes: Option<Vec<SeedNode>>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Wss,
    Ws,
    Dumb,
    Rtc,
}

/*
impl From<crate::network::Protocol> for Protocol {
    fn from(p: crate::network::Protocol) -> Self {
        match p {
            crate::network::Protocol::Wss => Self::Wss,
            crate::network::Protocol::Ws => Self::Ws,
            crate::network::Protocol::Dumb => Self::Dumb,
            crate::network::Protocol::Rtc => Self::Rtc,
        }
    }
}
*/

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedNode {
    host: String,
    port: u16,
    public_key: Option<String>,
}



#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsSettings {
    cert: String,
    key: String,
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusSettings {
    #[serde(rename = "type")]
    node_type: Option<NodeType>,
    network: Option<Network>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Full,
    Light,
    Nano,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Main,
    Test,
    Dev,
}

/*
impl From<crate::consensus::networks::NetworkId> for Network {
    fn from(n: crate::consensus::networks::NetworkId) -> Self {
        match n {
            crate::consensus::networks::NetworkId::Test => Self::Test,
            crate::consensus::networks::NetworkId::Dev => Self::Dev,
            crate::consensus::networks::NetworkId::Bounty => Self::Bounty,
            crate::consensus::networks::NetworkId::Dummy => Self::Dummy,
            crate::consensus::networks::NetworkId::Main => Self::Main,
        }
    }
}
*/


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinerSettings {
    enabled: Option<bool>,
    threads: Option<u64>,
    throttle_after: Option<u64>, // TODO: Fix that this can take "infinity"
    throttle_wait: Option<u64>,
    extra_data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PoolMiningSettings {
    enabled: Option<bool>,
    host: Option<String>,
    port: Option<u16>,
    mode: Option<String>,
    // TODO: This only accepts String -> String.
    device_data: Option<HashMap<String, String>>
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcServerSettings {
    enabled: Option<bool>,
    port: Option<u16>,
    corsdomain: Option<Vec<String>>,
    allowip: Option<Vec<String>>,
    methods: Option<Vec<String>>,
    username: Option<String>,
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UiServerSettings {
    enabled: Option<bool>,
    port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct MetricsServerSettings {
    enabled: Option<bool>,
    port: Option<u16>,
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WalletSettings {
    seed: Option<String>,
    address: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReverseProxySettings {
    enabled: Option<bool>,
    port: Option<u16>,
    address: Option<String>,
    header: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LogSettings {
    level: Option<String>,
    tags: Option<HashMap<String, String>>,
    statistics: Option<u64>,
}
