use std::str::FromStr;

use futures::StreamExt;
use gloo_timers::future::IntervalStream;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use log::level_filters::LevelFilter;

pub use nimiq::{
    client::Consensus,
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::{ConfigFile, LogSettings, Seed, SyncMode},
    error::Error,
    extras::{panic::initialize_panic_reporting, web_logging::initialize_web_logging},
};

use beserial::{Deserialize, Serialize};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::ConsensusEvent;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{Network, NetworkEvent},
    Multiaddr,
};

use crate::peer_info::PeerInfo;
use crate::transaction::Transaction;
use crate::utils::{from_network_id, to_network_id};

mod address;
mod key_pair;
mod peer_info;
mod private_key;
mod public_key;
mod signature;
mod signature_proof;
mod transaction;
mod utils;

/// Use this to provide initialization-time configuration to the Client.
/// This is a simplified version of the configuration that is used for regular nodes,
/// since not all configuration knobs are available when running inside a browser.
#[wasm_bindgen]
pub struct ClientConfiguration {
    seed_nodes: Vec<String>,
    log_level: String,
}

impl Default for ClientConfiguration {
    fn default() -> Self {
        Self {
            seed_nodes: vec![],
            log_level: "info".to_string(),
        }
    }
}

#[wasm_bindgen]
impl ClientConfiguration {
    /// Creates a default client configuration that can be used to change the client's configuration.
    ///
    /// Use its `instantiateClient()` method to launch the client and connect to the network.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        ClientConfiguration::default()
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

    /// Instantiates a client from this configuration builder.
    #[wasm_bindgen(js_name = instantiateClient)]
    pub async fn instantiate_client(&self) -> Client {
        Client::create(self).await
    }
}

/// Nimiq Albatross client that runs in browsers via WASM and is exposed to Javascript.
///
/// Usage:
/// ```js
/// import init, * as Nimiq from "./pkg/nimiq_web_client.js";
///
/// init().then(async () => {
///     const configBuilder = Nimiq.ClientConfiguration.builder();
///     const client = await configBuilder.instantiateClient();
///     // ...
/// });
/// ```
#[wasm_bindgen]
pub struct Client {
    #[wasm_bindgen(skip)]
    pub inner: nimiq::client::Client,

    /// The network ID that the client is connecting to.
    #[wasm_bindgen(readonly, js_name = networkId)]
    pub network_id: u8,
}

#[wasm_bindgen]
impl Client {
    /// Creates a new WebClient that automatically starts connecting to the network.
    pub async fn create(web_config: &ClientConfiguration) -> Client {
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

        let network_id = from_network_id(config.network_id);

        log::debug!(?config, "Final configuration");

        // Create client from config.
        log::info!("Initializing light client");
        let mut client = nimiq::client::Client::from_config(
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

        Client {
            inner: client,
            network_id,
        }
    }

    /// Starts the consensus event stream.
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

    /// Starts the blockchain head event stream.
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

    /// Starts the peer event stream.
    ///
    /// Updates are emitted via `__wasm_imports.peer_listener`.
    pub async fn subscribe_peers(&self) {
        let network = self.inner.network();
        let mut network_events = network.subscribe_events();

        loop {
            match network_events.next().await {
                Some(Ok(NetworkEvent::PeerJoined(peer_id, peer_info))) => {
                    peer_listener(
                        "joined",
                        peer_id.to_string(),
                        network.peer_count(),
                        Some(PeerInfo::from_native(peer_info)),
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

    /// Starts an interval to report statistics.
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

    /// Returns if the client currently has consensus with the network.
    #[wasm_bindgen(js_name = isEstablished)]
    pub fn is_established(&self) -> bool {
        self.inner.consensus_proxy().is_established()
    }

    /// Returns the block number of the current blockchain head.
    #[wasm_bindgen(js_name = blockNumber)]
    pub fn block_number(&self) -> u32 {
        self.inner.blockchain_head().block_number()
    }

    /// Sends a transaction to the network. This method does not check if the
    /// transaction gets included into a block.
    ///
    /// Throws in case of a networking error.
    #[wasm_bindgen(js_name = sendTransaction)]
    pub async fn send_transaction(&self, transaction: &Transaction) -> Result<(), JsError> {
        transaction.verify(Some(self.network_id))?;

        self.inner
            .consensus_proxy()
            .send_transaction(transaction.native_ref().clone())
            .await?;

        Ok(())
    }

    /// Sends a serialized transaction to the network. This method does not check if the
    /// transaction gets included into a block.
    ///
    /// Throws when the transaction cannot be parsed from the byte array or in case of a networking error.
    #[wasm_bindgen(js_name = sendRawTransaction)]
    pub async fn send_raw_transaction(&self, raw_tx: &[u8]) -> Result<(), JsError> {
        let tx = nimiq_transaction::Transaction::deserialize_from_vec(raw_tx)?;

        tx.verify(to_network_id(self.network_id).ok().unwrap())?;

        self.inner.consensus_proxy().send_transaction(tx).await?;

        Ok(())
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
