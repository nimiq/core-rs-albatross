#[cfg(any(feature = "client", feature = "primitives"))]
use std::str::FromStr;

use nimiq_primitives::networks::NetworkId;
use tsify::Tsify;
use wasm_bindgen::prelude::wasm_bindgen;
#[cfg(any(feature = "client", feature = "primitives"))]
use wasm_bindgen::prelude::JsError;
#[cfg(feature = "primitives")]
use wasm_bindgen::prelude::JsValue;

/// Use this to provide initialization-time configuration to the Client.
/// This is a simplified version of the configuration that is used for regular nodes,
/// since not all configuration knobs are available when running inside a browser.
#[derive(Debug)]
#[wasm_bindgen]
pub struct ClientConfiguration {
    #[wasm_bindgen(skip)]
    pub network_id: NetworkId,
    #[wasm_bindgen(skip)]
    pub seed_nodes: Vec<String>,
    #[wasm_bindgen(skip)]
    pub log_level: String,
    #[wasm_bindgen(skip)]
    pub only_secure_ws_connections: bool,
    #[wasm_bindgen(skip)]
    pub desired_peer_count: usize,
    #[wasm_bindgen(skip)]
    pub peer_count_max: usize,
    #[wasm_bindgen(skip)]
    pub peer_count_per_ip_max: usize,
    #[wasm_bindgen(skip)]
    pub peer_count_per_subnet_max: usize,
    #[wasm_bindgen(skip)]
    pub network_buffer_size: usize,
    #[wasm_bindgen(skip)]
    pub sync_mode: String,
    #[wasm_bindgen(skip)]
    pub num_initial_connections: usize,
}

#[cfg(any(feature = "client", feature = "primitives"))]
#[cfg_attr(feature = "primitives", derive(serde::Serialize))]
#[cfg_attr(feature = "client", derive(serde::Deserialize))]
#[cfg_attr(
    any(feature = "client", feature = "primitives"),
    derive(Tsify),
    serde(rename_all = "camelCase")
)]
pub struct PlainClientConfiguration {
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub network_id: Option<String>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub seed_nodes: Option<Vec<String>>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub log_level: Option<String>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub only_secure_ws_connections: Option<bool>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub desired_peer_count: Option<usize>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub peer_count_max: Option<usize>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub peer_count_per_ip_max: Option<usize>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub peer_count_per_subnet_max: Option<usize>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub network_buffer_size: Option<usize>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub sync_mode: Option<String>,
    #[cfg_attr(feature = "client", serde(skip_serializing_if = "Option::is_none"))]
    pub num_initial_connections: Option<usize>,
}

impl Default for ClientConfiguration {
    fn default() -> Self {
        Self {
            network_id: NetworkId::MainAlbatross,
            seed_nodes: vec![
                "/dns4/aurora.seed.nimiq.com/tcp/443/wss".to_string(),
                "/dns4/catalyst.seed.nimiq.network/tcp/443/wss".to_string(),
                "/dns4/cipher.seed.nimiq-network.com/tcp/443/wss".to_string(),
                "/dns4/eclipse.seed.nimiq.cloud/tcp/443/wss".to_string(),
                "/dns4/lumina.seed.nimiq.systems/tcp/443/wss".to_string(),
                "/dns4/nebula.seed.nimiq.com/tcp/443/wss".to_string(),
                "/dns4/nexus.seed.nimiq.network/tcp/443/wss".to_string(),
                "/dns4/polaris.seed.nimiq-network.com/tcp/443/wss".to_string(),
                "/dns4/photon.seed.nimiq.cloud/tcp/443/wss".to_string(),
                "/dns4/pulsar.seed.nimiq.systems/tcp/443/wss".to_string(),
                "/dns4/quasar.seed.nimiq.com/tcp/443/wss".to_string(),
                "/dns4/solstice.seed.nimiq.network/tcp/443/wss".to_string(),
                "/dns4/vortex.seed.nimiq.cloud/tcp/443/wss".to_string(),
                "/dns4/zenith.seed.nimiq.systems/tcp/443/wss".to_string(),
            ],
            log_level: "info".to_string(),
            only_secure_ws_connections: true,
            desired_peer_count: 12,
            peer_count_max: 50,
            peer_count_per_ip_max: 10,
            peer_count_per_subnet_max: 10,
            sync_mode: "light".to_string(),
            num_initial_connections: 4,
            network_buffer_size: 1024,
        }
    }
}

#[cfg(feature = "primitives")]
#[wasm_bindgen]
impl ClientConfiguration {
    /// Creates a default client configuration that can be used to change the client's configuration.
    ///
    /// Use its `instantiateClient()` method to launch the client and connect to the network.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        ClientConfiguration::default()
    }

    /// Sets the network ID the client should use. Input is case-insensitive.
    ///
    /// Possible values are `'MainAlbatross' | 'TestAlbatross' | 'DevAlbatross'`.
    /// Default is `'MainAlbatross'`.
    pub fn network(&mut self, network: String) -> Result<(), JsError> {
        self.network_id = NetworkId::from_str(&network)?;
        Ok(())
    }

    /// Sets the list of seed nodes that are used to connect to the Nimiq Albatross network.
    ///
    /// Each array entry must be a proper Multiaddr format string.
    #[wasm_bindgen(js_name = seedNodes)]
    #[allow(clippy::boxed_local)]
    pub fn seed_nodes(&mut self, seeds: Box<[JsValue]>) {
        self.seed_nodes = seeds
            .iter()
            .map(|seed| serde_wasm_bindgen::from_value(seed.clone()).unwrap())
            .collect::<Vec<String>>();
    }

    /// Sets the log level that is used when logging to the console.
    ///
    /// Possible values are `'trace' | 'debug' | 'info' | 'warn' | 'error'`.
    /// Default is `'info'`.
    #[wasm_bindgen(js_name = logLevel)]
    pub fn log_level(&mut self, log_level: String) {
        self.log_level = log_level.to_lowercase();
    }

    /// Sets whether the client should only connect to secure WebSocket connections.
    /// Default is `true`.
    #[wasm_bindgen(js_name = onlySecureWsConnections)]
    pub fn only_secure_ws_connections(&mut self, only_secure_ws_connections: bool) {
        self.only_secure_ws_connections = only_secure_ws_connections;
    }

    /// Sets the desired number of peers the client should try to connect to.
    /// Default is `12`.
    #[wasm_bindgen(js_name = desiredPeerCount)]
    pub fn desired_peer_count(&mut self, desired_peer_count: usize) {
        self.desired_peer_count = desired_peer_count;
    }

    /// Sets the maximum number of peers the client should connect to.
    /// Default is `50`.
    #[wasm_bindgen(js_name = peerCountMax)]
    pub fn peer_count_max(&mut self, peer_count_max: usize) {
        self.peer_count_max = peer_count_max;
    }

    /// Sets the maximum number of peers the client should connect to per IP address.
    /// Default is `10`.
    #[wasm_bindgen(js_name = peerCountPerIpMax)]
    pub fn peer_count_per_ip_max(&mut self, peer_count_per_ip_max: usize) {
        self.peer_count_per_ip_max = peer_count_per_ip_max;
    }

    /// Sets the maximum number of peers the client should connect to per subnet.
    /// Default is `10`.
    #[wasm_bindgen(js_name = peerCountPerSubnetMax)]
    pub fn peer_count_per_subnet_max(&mut self, peer_count_per_subnet_max: usize) {
        self.peer_count_per_subnet_max = peer_count_per_subnet_max;
    }

    /// Sets the maximum network buffer size, which should be greater than 0
    /// Default is `1024`.
    #[wasm_bindgen(js_name = networkBufferSize)]
    pub fn network_buffer_size(&mut self, network_buffer_size: usize) {
        if network_buffer_size == 0_usize {
            panic!("The network buffer size needs to be greater than 0")
        }
        self.network_buffer_size = network_buffer_size;
    }

    /// Sets the sync mode that shoud be used.
    /// Only "light" and "pico" are supported for web clients
    /// Default is "light"
    #[wasm_bindgen(js_name = syncMode)]
    pub fn sync_mode(&mut self, sync_mode: String) {
        self.sync_mode = sync_mode.to_lowercase();
    }

    // TODO: Find a way to make this method work, maybe by using the synthetic Client from the main thread as an import?
    // /// Instantiates a client from this configuration builder.
    // #[wasm_bindgen(js_name = instantiateClient)]
    // pub async fn instantiate_client(&self) -> Client {
    //     match Client::create(&self.build()).await {
    //         Ok(client) => client,
    //         Err(_) => unreachable!(),
    //     }
    // }

    /// Returns a plain configuration object to be passed to `Client.create`.
    pub fn build(&self) -> PlainClientConfigurationType {
        serde_wasm_bindgen::to_value(&PlainClientConfiguration {
            network_id: Some(self.network_id.to_string()),
            seed_nodes: Some(self.seed_nodes.clone()),
            log_level: Some(self.log_level.clone()),
            only_secure_ws_connections: Some(self.only_secure_ws_connections),
            desired_peer_count: Some(self.desired_peer_count),
            peer_count_max: Some(self.peer_count_max),
            peer_count_per_ip_max: Some(self.peer_count_per_ip_max),
            peer_count_per_subnet_max: Some(self.peer_count_per_subnet_max),
            sync_mode: Some(self.sync_mode.clone()),
            num_initial_connections: Some(self.num_initial_connections),
            network_buffer_size: Some(self.network_buffer_size),
        })
        .unwrap()
        .into()
    }
}

#[cfg(feature = "client")]
impl TryFrom<PlainClientConfiguration> for ClientConfiguration {
    type Error = JsError;

    fn try_from(config: PlainClientConfiguration) -> Result<ClientConfiguration, JsError> {
        let mut client_config = ClientConfiguration::default();

        if let Some(network_id) = config.network_id {
            client_config.network_id = NetworkId::from_str(&network_id)
                .map_err(|err| JsError::new(&format!("Invalid network ID: {err}")))?;
        }

        if let Some(seed_nodes) = config.seed_nodes {
            client_config.seed_nodes = seed_nodes;
        }

        if let Some(log_level) = config.log_level {
            client_config.log_level = log_level;
        }

        if let Some(only_secure_ws_connections) = config.only_secure_ws_connections {
            client_config.only_secure_ws_connections = only_secure_ws_connections;
        }

        if let Some(desired_peer_count) = config.desired_peer_count {
            client_config.desired_peer_count = desired_peer_count;
        }

        if let Some(peer_count_max) = config.peer_count_max {
            client_config.peer_count_max = peer_count_max;
        }

        if let Some(peer_count_per_ip_max) = config.peer_count_per_ip_max {
            client_config.peer_count_per_ip_max = peer_count_per_ip_max;
        }

        if let Some(peer_count_per_subnet_max) = config.peer_count_per_subnet_max {
            client_config.peer_count_per_subnet_max = peer_count_per_subnet_max;
        }

        if let Some(network_buffer_size) = config.network_buffer_size {
            client_config.network_buffer_size = network_buffer_size;
        }

        if let Some(sync_mode) = config.sync_mode {
            // Only "pico" and "light" sync modes are supported for web clients
            if !(sync_mode == "pico" || sync_mode == "light") {
                return Err(JsError::new(&format!("Invalid sync mode: {sync_mode}")));
            }

            client_config.sync_mode = sync_mode;
        }

        Ok(client_config)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainClientConfiguration")]
    pub type PlainClientConfigurationType;
}
