use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;

use futures::StreamExt;
#[cfg(feature = "metrics")]
use lazy_static::lazy_static;
#[cfg(feature = "metrics")]
use prometheus::{IntGauge, Registry};
use rand::{thread_rng, RngCore};
use structopt::StructOpt;
#[cfg(feature = "metrics")]
use warp::{Filter, Rejection, Reply};

use nimiq::client::ConsensusProxy;
pub use nimiq::{
    client::{Client, Consensus},
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::ConfigFile,
    error::Error,
    extras::{
        deadlock::initialize_deadlock_detection,
        logging::{initialize_logging, log_error_cause_chain},
        panic::initialize_panic_reporting,
    },
};
use nimiq_block::BlockType;
use nimiq_blockchain::{AbstractBlockchain, BlockchainEvent};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::mempool::Mempool;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;

#[cfg(feature = "metrics")]
lazy_static! {
    pub static ref REGISTRY: Registry = Registry::new();
    pub static ref BLOCK_NUMBER: IntGauge =
        IntGauge::new("block_number", "Block Number").expect("metric couldn't be created");
    pub static ref TPS: IntGauge = IntGauge::new("spammer_tps", "Spammer transactions per second")
        .expect("metric couldn't be created");
    pub static ref BLOCK_TIME: IntGauge = IntGauge::new("block_time", "Spammer average block time")
        .expect("metric couldn't be created");
}

#[derive(Debug, StructOpt)]
#[structopt(rename_all = "kebab")]
pub struct SpammerCommandLine {
    /// Use a custom configuration file.
    ///
    /// # Examples
    ///
    /// * `nimiq-spammer --config ~/.nimiq/client-albatross.toml`
    ///
    #[structopt(long, short = "c")]
    pub config: Option<PathBuf>,

    /// Transactions per block to generate.
    ///
    /// * `nimiq-spammer --tpb 724`
    #[structopt(long, short = "t")]
    pub tpb: Option<u32>,
}

impl SpammerCommandLine {
    pub fn from_args() -> Self {
        <Self as StructOpt>::from_args()
    }
}

impl FromIterator<String> for SpammerCommandLine {
    /// Load command line from command line arguments (std::env::args)
    fn from_iter<I: IntoIterator<Item = String>>(args: I) -> Self {
        <Self as StructOpt>::from_iter(args)
    }
}

#[derive(Clone)]
struct StatsExert {
    pub time: std::time::Duration,
    pub is_micro: bool,
    pub tx_count: usize,
}

const UNIT_KEY: &str = "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";
const DEV_KEY: &str = "1ef7aad365c195462ed04c275d47189d5362bbfe36b5e93ce7ba2f3add5f439b";

#[cfg(feature = "metrics")]
async fn metrics_handler() -> Result<impl Reply, Rejection> {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&REGISTRY.gather(), &mut buffer) {
        eprintln!("could not encode custom metrics: {}", e);
    };
    let mut res = match String::from_utf8(buffer.clone()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("custom metrics could not be from_utf8'd: {}", e);
            String::default()
        }
    };
    buffer.clear();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&prometheus::gather(), &mut buffer) {
        eprintln!("could not encode prometheus metrics: {}", e);
    };
    let res_custom = match String::from_utf8(buffer.clone()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("prometheus metrics could not be from_utf8'd: {}", e);
            String::default()
        }
    };
    buffer.clear();

    res.push_str(&res_custom);
    Ok(res)
}

#[cfg(feature = "metrics")]
async fn register_custom_metrics() {
    REGISTRY
        .register(Box::new(TPS.clone()))
        .expect("collector couldn't be registered");
    REGISTRY
        .register(Box::new(BLOCK_TIME.clone()))
        .expect("collector couldn't registered");
    REGISTRY
        .register(Box::new(BLOCK_NUMBER.clone()))
        .expect("collector couldn't be registered");

    let metrics_route = warp::path!("metrics").and_then(metrics_handler);

    warp::serve(metrics_route).run(([0, 0, 0, 0], 9501)).await;
}

#[cfg(feature = "metrics")]
async fn update_metric_counters(block_number: i64, block_time: i64, spammer_tps: i64) {
    TPS.set(spammer_tps);
    BLOCK_TIME.set(block_time);
    BLOCK_NUMBER.set(block_number);
}

async fn main_inner() -> Result<(), Error> {
    // Initialize deadlock detection
    initialize_deadlock_detection();

    // Parse command line.
    let spammer_command_line = SpammerCommandLine::from_args();
    log::trace!("Command line: {:#?}", spammer_command_line);

    let command_line = CommandLine {
        config: spammer_command_line.config,
        log_level: None,
        log_tags: None,
        passive: false,
        sync_mode: None,
        network: None,
    };

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    log::trace!("Config file: {:#?}", config_file);

    // Initialize logging with config values.
    initialize_logging(Some(&command_line), Some(&config_file.log))?;

    // Initialize panic hook.
    initialize_panic_reporting();

    // Register metrics
    #[cfg(feature = "metrics")]
    tokio::spawn(register_custom_metrics());

    // Create config builder and apply command line and config file.
    // You usually want the command line to override config settings, so the order is important.
    let mut builder = ClientConfig::builder();
    builder.config_file(&config_file)?;
    builder.command_line(&command_line)?;

    // Finalize config.
    let config = builder.build()?;
    log::debug!("Final configuration: {:#?}", config);

    // Clone config for RPC and metrics server
    let rpc_config = config.rpc_server.clone();

    // Get the private key used to sign the transactions (the associated address must have funds).
    let validator_settings = &config_file
        .validator
        .expect("A spammer is always a validator (it needs a mempool)");
    let private_key = match config.network_id {
        NetworkId::UnitAlbatross => UNIT_KEY,
        // First try to get it from the "fee_key" field in the config file, if that's not set, then use the hardcoded default.
        NetworkId::DevAlbatross => validator_settings.fee_key.as_deref().unwrap_or(DEV_KEY),
        _ => panic!("Unsupported network"),
    };

    let key_pair = KeyPair::from(PrivateKey::from_str(private_key).unwrap());
    log::info!(
        "Funds for txs will come from this address: {}",
        Address::from(&key_pair)
    );

    // Create client from config.
    log::info!("Initializing client");
    let mut client: Client = Client::from_config(config).await?;
    log::info!("Client initialized");

    // Initialize RPC server
    if let Some(rpc_config) = rpc_config {
        use nimiq::extras::rpc_server::initialize_rpc_server;
        let rpc_server = initialize_rpc_server(&client, rpc_config, client.wallet_store())
            .expect("Failed to initialize RPC server");
        tokio::spawn(async move { rpc_server.run().await });
    }

    // Start consensus.
    let consensus = client.consensus().unwrap();

    let mut bc_events = {
        let mut bc = consensus.blockchain.write();
        bc.notifier.as_stream()
    };

    log::info!("Spawning consensus");
    tokio::spawn(consensus);
    let consensus = client.consensus_proxy();

    // Start Spammer
    let mempool = if let Some(validator) = client.validator() {
        log::info!("Spawning spammer");
        let mempool = std::sync::Arc::clone(&validator.mempool);
        tokio::spawn(validator);
        mempool
    } else {
        panic!("Could not start spammer");
    };

    let rolling_window = 32usize;

    let mut stat_exerts: VecDeque<StatsExert> = VecDeque::new();
    let mut tx_count_total = 0usize;
    let mut micro_block_count = 0usize;

    let mut count = 500;
    if let Some(tpb) = spammer_command_line.tpb {
        count = tpb as usize;
    }

    log::info!("Spammer configured to generate {} tx/block", count);

    loop {
        while let Some(event) = bc_events.next().await {
            let hash = match event {
                BlockchainEvent::Extended(hash) => Some(hash),
                BlockchainEvent::EpochFinalized(hash) => Some(hash),
                BlockchainEvent::Finalized(hash) => Some(hash),
                _ => None,
            };
            if let Some(hash) = hash {
                let block = {
                    let blockchain = consensus.blockchain.read();
                    blockchain
                        .get_block(&hash, true, None)
                        .expect("Failed to get latest block")
                };

                log::info!("\n");
                if consensus.is_established() {
                    spam(
                        std::sync::Arc::clone(&mempool),
                        consensus.clone(),
                        key_pair.clone(),
                        count,
                    )
                    .await;
                    log::info!("\tSent {} transactions to the network.\n", count);
                }

                let time = std::time::Duration::from_millis(block.header().timestamp());
                let tx_count = block.transactions().map(|txs| txs.len()).unwrap_or(0);
                let mempool_count = mempool.num_transactions();

                log::info!(
                    "Blockchain extended to #{}.{}",
                    block.block_number(),
                    block.view_number()
                );
                if consensus.is_established() {
                    log::info!("\t- block contains: {} tx", tx_count);
                    log::info!("\t- mempool contains: {} tx", mempool_count);
                }

                tx_count_total += tx_count;

                let is_micro = block.ty() == BlockType::Micro;
                if is_micro {
                    micro_block_count += 1;
                }

                let newest_block = StatsExert {
                    time,
                    is_micro,
                    tx_count,
                };
                stat_exerts.push_back(newest_block.clone());

                if stat_exerts.len() == rolling_window {
                    let oldest_block = stat_exerts.pop_front().unwrap();
                    // len is now rolling_window - 1

                    // get average block time:
                    let diff = newest_block
                        .time
                        .checked_sub(oldest_block.time)
                        .expect("This should work");
                    let av_block_time = diff
                        .checked_div(rolling_window as u32)
                        .expect("This should work, too");

                    // get average tx per block:
                    let av_tx = tx_count_total.checked_div(micro_block_count).unwrap_or(0);

                    let tps = tx_count_total as f32 / diff.as_secs_f32();

                    #[cfg(feature = "metrics")]
                    update_metric_counters(
                        block.block_number() as i64,
                        av_block_time.as_millis().try_into().unwrap(),
                        tps as i64,
                    )
                    .await;

                    log::info!("Average over the last {} blocks:", rolling_window);
                    log::info!("\t- block time: {:?}", av_block_time);
                    log::info!("\t- tx per block: {:?}", av_tx);
                    log::info!("\t- tx per second: {:?}", tps);

                    tx_count_total -= oldest_block.tx_count;
                    if oldest_block.is_micro {
                        micro_block_count -= 1;
                    }
                }
            }
        }
    }
}

async fn spam(
    mempool: std::sync::Arc<Mempool>,
    consensus: ConsensusProxy,
    key_pair: KeyPair,
    count: usize,
) {
    let (number, net_id) = {
        let blockchain = consensus.blockchain.read();
        (blockchain.block_number(), blockchain.network_id)
    };
    tokio::task::spawn_blocking(move || {
        let txs = generate_transactions(&key_pair, number, net_id, count);

        for tx in txs {
            let consensus1 = consensus.clone();
            let mp = std::sync::Arc::clone(&mempool);
            tokio::spawn(async move {
                if let Err(e) = mp.add_transaction(tx.clone()).await {
                    log::warn!("Mempool rejected transaction: {:?} - {:#?}", e, tx);
                }
                if let Err(e) = consensus1.send_transaction(tx).await {
                    log::warn!("Failed to send transaction: {:?}", e);
                }
            });
        }
    })
    .await
    .expect("spawn_blocking() panicked");
}

fn generate_transactions(
    key_pair: &KeyPair,
    start_height: u32,
    network_id: NetworkId,
    count: usize,
) -> Vec<Transaction> {
    let mut txs = Vec::new();

    let mut rng = thread_rng();
    for _ in 0..count {
        let mut bytes = [0u8; 20];
        rng.fill_bytes(&mut bytes);
        let recipient = Address::from(bytes);

        let tx = TransactionBuilder::new_basic(
            key_pair,
            recipient,
            Coin::from_u64_unchecked(1),
            Coin::from_u64_unchecked(200),
            start_height,
            network_id,
        )
        .unwrap();
        txs.push(tx);
    }

    txs
}

#[tokio::main]
async fn main() {
    if let Err(e) = main_inner().await {
        log_error_cause_chain(&e);
    }
}
