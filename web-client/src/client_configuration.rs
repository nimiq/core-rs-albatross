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
}

impl Default for ClientConfiguration {
    fn default() -> Self {
        Self {
            network_id: NetworkId::TestAlbatross,
            seed_nodes: vec!["/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss".to_string()],
            log_level: "info".to_string(),
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
    /// Possible values are `'TestAlbatross' | 'DevAlbatross'`.
    /// Default is `'TestAlbatross'`.
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
                .map_err(|err| JsError::new(&format!("Invalid network ID: {}", err)))?;
        }

        if let Some(seed_nodes) = config.seed_nodes {
            client_config.seed_nodes = seed_nodes;
        }

        if let Some(log_level) = config.log_level {
            client_config.log_level = log_level;
        }

        Ok(client_config)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainClientConfiguration")]
    pub type PlainClientConfigurationType;
}
