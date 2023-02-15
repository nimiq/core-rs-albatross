use std::str::FromStr;

use futures::StreamExt;
use gloo_timers::future::IntervalStream;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use log::level_filters::LevelFilter;

pub use nimiq::{
    client::{Client, Consensus},
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::{ConfigFile, LogSettings, Seed, SyncMode},
    error::Error,
    extras::{panic::initialize_panic_reporting, web_logging::initialize_web_logging},
};

use beserial::Serialize;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::ConsensusEvent;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{Network, NetworkEvent},
    peer_info::{NodeType, Services},
    Multiaddr,
};

pub mod PrivateKey;

/// Peer information that is exposed to Javascript
/// This information is a translated form of what is sent by the Network upon
/// `PeerJoined` events.
#[wasm_bindgen]
pub struct PeerInfo {
    /// Address of the Peer in `Multiaddr` format
    address: String,
    /// Type of node (full, history or light)
    node_type: String,
}

#[wasm_bindgen]
impl PeerInfo {
    pub fn new(address: String, node_type: String) -> Self {
        Self { address, node_type }
    }

    /// Gets the address
    #[wasm_bindgen(js_name = getAddress)]
    pub fn get_address(&self) -> String {
        self.address.clone()
    }

    /// Gets the node type
    #[wasm_bindgen(js_name = getNodeType)]
    pub fn get_node_type(&self) -> String {
        self.node_type.clone()
    }
}

/// Struct that is used to provide initialization-time configuration to the WebClient
/// This a simplified version of the configuration that is used for regular nodes,
/// since not all configuration knobs are available when running inside a browser.
/// For instance, only the light sync mechanism is supported in the browser
///
#[wasm_bindgen]
pub struct WebClientConfiguration {
    /// The list of seeds nodes that are used to connect to the 2.0 network.
    /// This string should be a proper Multiaddr format string.
    seed_nodes: Vec<String>,
    /// The log level that is used when logging to the web console.
    /// The same log levels (trace, debug, info, etc) that are supported by the regular client.
    log_level: String,
}

#[wasm_bindgen]
impl WebClientConfiguration {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::boxed_local)]
    pub fn new(seed_nodes: Box<[JsValue]>, log_level: String) -> WebClientConfiguration {
        let seed_nodes = seed_nodes
            .iter()
            .map(|seed| serde_wasm_bindgen::from_value(seed.clone()).unwrap())
            .collect::<Vec<String>>();

        WebClientConfiguration {
            seed_nodes,
            log_level,
        }
    }
}

/// Nimiq Albatross client that runs in browsers via WASM and is exposed to Javascript.
///
/// Usage:
/// ```js
/// import init, { WebClient } from "./pkg/nimiq_web_client.js";
///
/// init().then(async () => {
///     const client = await WebClient.create();
///     // ...
/// });
/// ```
#[wasm_bindgen]
pub struct WebClient {
    #[wasm_bindgen(skip)]
    pub inner: Client,
}

#[wasm_bindgen]
impl WebClient {
    /// Create a new WebClient that automatically starts connecting to the network.
    pub async fn create(web_config: WebClientConfiguration) -> WebClient {
        let log_settings = LogSettings {
            level: Some(LevelFilter::from_str(web_config.log_level.as_str()).unwrap()),
            ..Default::default()
        };

        // Initialize logging with config values.
        initialize_web_logging(Some(&log_settings)).expect("Web logging initialization failed");

        // Initialize panic hook.
        initialize_panic_reporting();

        // Create config builder.
        let mut builder = ClientConfig::builder();

        // Finalize config.
        let mut config = builder
            .volatile()
            .light()
            .build()
            .expect("Build configuration failed");

        // Set the seed nodes
        let seed_nodes = web_config
            .seed_nodes
            .iter()
            .map(|seed| Seed {
                address: Multiaddr::from_str(seed).unwrap(),
            })
            .collect::<Vec<Seed>>();

        config.network.seeds = seed_nodes;

        log::debug!(?config, "Final configuration");

        // Create client from config.
        log::info!("Initializing light client");
        let mut client: Client = Client::from_config(
            config,
            Box::new(|fut| {
                spawn_local(fut);
            }),
        )
        .await
        .expect("Client initialization failed");
        log::info!("Web client initialized");

        // Start consensus.
        let consensus = client.take_consensus().unwrap();
        log::info!("Spawning consensus");
        spawn_local(consensus);

        let zkp_component = client.take_zkp_component().unwrap();
        spawn_local(zkp_component);

        WebClient { inner: client }
    }

    /// Start the consensus event stream.
    ///
    /// Updates are emitted via `__wasm_imports.consensus_listener`.
    pub async fn subscribe_consensus(&self) {
        let mut consensus_events = self.inner.consensus_proxy().subscribe_events();

        loop {
            match consensus_events.next().await {
                Some(Ok(ConsensusEvent::Established)) => {
                    consensus_listener(true);
                }
                Some(Ok(ConsensusEvent::Lost)) => {
                    consensus_listener(false);
                }
                Some(Err(_error)) => {} // Ignore stream errors
                None => {
                    break;
                }
            }
        }
    }

    /// Start the blockchain head event stream.
    ///
    /// Updates are emitted via `__wasm_imports.block_listener`.
    pub async fn subscribe_blocks(&self) {
        let blockchain = self.inner.consensus_proxy().blockchain;
        let mut blockchain_events = blockchain.read().notifier_as_stream();

        fn emit_block(
            blockchain: &BlockchainProxy,
            ty: &str,
            hash: Blake2bHash,
            rebranch_length: Option<usize>,
        ) {
            if let Ok(block) = blockchain.read().get_block(&hash, false) {
                block_listener(ty, block.header().serialize_to_vec(), rebranch_length);
            }
        }

        loop {
            match blockchain_events.next().await {
                Some(BlockchainEvent::Extended(hash)) => {
                    emit_block(&blockchain, "extended", hash, None);
                }
                Some(BlockchainEvent::HistoryAdopted(hash)) => {
                    emit_block(&blockchain, "history-adopted", hash, None);
                }
                Some(BlockchainEvent::EpochFinalized(hash)) => {
                    emit_block(&blockchain, "epoch-finalized", hash, None);
                }
                Some(BlockchainEvent::Finalized(hash)) => {
                    emit_block(&blockchain, "finalized", hash, None);
                }
                Some(BlockchainEvent::Rebranched(_, new_chain)) => {
                    emit_block(
                        &blockchain,
                        "rebranched",
                        new_chain.last().unwrap().to_owned().0,
                        Some(new_chain.len()),
                    );
                }
                None => {
                    break;
                }
            }
        }
    }

    /// Start the peer event stream.
    ///
    /// Updates are emitted via `__wasm_imports.peer_listener`.
    pub async fn subscribe_peers(&self) {
        let network = self.inner.network();
        let mut network_events = network.subscribe_events();

        loop {
            match network_events.next().await {
                Some(Ok(NetworkEvent::PeerJoined(peer_id, peer_info))) => {
                    let node_type = if peer_info
                        .get_services()
                        .contains(Services::provided(NodeType::History))
                    {
                        "History"
                    } else if peer_info
                        .get_services()
                        .contains(Services::provided(NodeType::Full))
                    {
                        "Full"
                    } else {
                        "Light"
                    };
                    let peer_info =
                        PeerInfo::new(peer_info.get_address().to_string(), node_type.to_string());
                    peer_listener(
                        "joined",
                        peer_id.to_string(),
                        network.peer_count(),
                        Some(peer_info),
                    );
                }
                Some(Ok(NetworkEvent::PeerLeft(peer_id))) => {
                    peer_listener("left", peer_id.to_string(), network.peer_count(), None);
                }
                Some(Err(_error)) => {} // Ignore stream errors
                None => {
                    break;
                }
            }
        }
    }

    /// Start an interval to report statistics.
    ///
    /// Updates are emitted via `__wasm_imports.statistics_listener`.
    pub async fn subscribe_statistics(&self) {
        let statistics_interval = LogSettings::default_statistics_interval();

        if statistics_interval == 0 {
            return;
        }

        // Run periodically
        let interval = IntervalStream::new(statistics_interval as u32 * 1000);

        let consensus = self.inner.consensus_proxy();

        interval
            .for_each(|_| async {
                match self.inner.network().network_info().await {
                    Ok(network_info) => {
                        let head = self.inner.blockchain_head();

                        log::debug!(
                            block_number = head.block_number(),
                            num_peers = network_info.num_peers(),
                            status = consensus.is_established(),
                            "Consensus status: {:?} - Head: #{}- {}",
                            consensus.is_established(),
                            head.block_number(),
                            head.hash(),
                        );

                        statistics_listener(
                            consensus.is_established(),
                            head.block_number(),
                            network_info.num_peers(),
                        );
                    }
                    Err(err) => {
                        log::error!("Error retrieving NetworkInfo: {:?}", err);
                    }
                };
            })
            .await;
    }
}

#[wasm_bindgen]
extern "C" {
    /// Imported Javascript function to receive consensus status
    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn consensus_listener(established: bool);

    /// Imported Javascript function to receive blockchain head updates
    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn block_listener(ty: &str, serialized_block: Vec<u8>, rebranch_length: Option<usize>);

    /// Imported Javascript function to receive peer updates
    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn peer_listener(ty: &str, peer_id: String, num_peers: usize, peer_info: Option<PeerInfo>);

    /// Imported Javascript function to receive statistics
    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn statistics_listener(established: bool, block_number: u32, num_peers: usize);
}
