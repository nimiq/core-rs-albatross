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
use nimiq_network_interface::network::{Network, NetworkEvent};
use nimiq_network_libp2p::Multiaddr;

#[wasm_bindgen]
pub struct WebClient {
    #[wasm_bindgen(skip)]
    pub inner: Client,
}

#[wasm_bindgen]
impl WebClient {
    pub async fn create() -> WebClient {
        let log_settings = LogSettings {
            level: Some(LevelFilter::DEBUG),
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

        let seed = Seed {
            address: Multiaddr::from_str("/dns4/seed1.v2.nimiq-testnet.com/tcp/8443/ws").unwrap(),
        };
        config.network.seeds = vec![seed];

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

    pub async fn subscribe_peers(&self) {
        let network = self.inner.network();
        let mut network_events = network.subscribe_events();

        loop {
            match network_events.next().await {
                Some(Ok(NetworkEvent::PeerJoined(peer_id))) => {
                    peer_listener("joined", peer_id.to_string(), network.peer_count());
                }
                Some(Ok(NetworkEvent::PeerLeft(peer_id))) => {
                    peer_listener("left", peer_id.to_string(), network.peer_count());
                }
                Some(Err(_error)) => {} // Ignore stream errors
                None => {
                    break;
                }
            }
        }
    }

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
    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn consensus_listener(established: bool);

    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn block_listener(ty: &str, serialized_block: Vec<u8>, rebranch_length: Option<usize>);

    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn peer_listener(ty: &str, peer_id: String, num_peers: usize);

    #[wasm_bindgen(js_namespace = __wasm_imports)]
    fn statistics_listener(established: bool, block_number: u32, num_peers: usize);
}
