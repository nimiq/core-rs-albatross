use std::collections::VecDeque;

use futures::StreamExt;
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
use nimiq_block::BlockBody;
use nimiq_blockchain::{AbstractBlockchain, BlockchainEvent};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::mempool::Mempool;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;
use rand::{thread_rng, RngCore};
use std::str::FromStr;

use std::path::PathBuf;
use structopt::StructOpt;

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

    /// Transactions per second to generate
    ///
    /// * `nimiq-spammer --tps 800`
    #[structopt(long, short = "t")]
    pub tps: Option<u32>,
}

impl SpammerCommandLine {
    pub fn from_args() -> Self {
        <Self as StructOpt>::from_args()
    }

    /// Load command line from command line arguments (std::env::args)
    pub fn from_iter<I: IntoIterator<Item = String>>(args: I) -> Self {
        <Self as StructOpt>::from_iter(args)
    }
}

#[derive(Clone)]
struct StatsExert {
    pub time: std::time::Duration,
    pub height: u32,
    pub tx_count: usize,
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

    // Create the "monitor" future which never completes to keep the client alive.
    // This closure is executed after the client has been initialized.
    // TODO Get rid of this. Make the Client a future/stream instead.
    let mut statistics_interval = config_file.log.statistics;
    let mut show_statistics = true;
    if statistics_interval == 0 {
        show_statistics = false;
    }
    statistics_interval = 1;

    let rolling_window = 32usize;

    let mut stat_exerts: VecDeque<StatsExert> = VecDeque::new();
    let mut tx_count_total = 0usize;

    let mut count = 150;

    if let Some(tps) = spammer_command_line.tps {
        count = tps as usize;
    }

    log::info!("Spammer configured to gerenerate {} tps", count);

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
                    blockchain.get_block(&hash, true, None)
                };

                let newest_block = if let Some(block) = block {
                    log::info!("\n");
                    if consensus.is_established() {
                        spam(std::sync::Arc::clone(&mempool), consensus.clone(), count).await;
                        log::info!(
                            "\tCreated {} transactions and send to the network.\n",
                            count
                        );
                    }

                    let time = std::time::Duration::from_millis(block.header().timestamp());

                    let tx_count = if let Some(BlockBody::Micro(body)) = block.body() {
                        body.transactions.len()
                    } else {
                        0
                    };

                    let mempool_count = mempool.num_transactions();

                    log::info!(
                        "Blockchain extended to #{}.{}",
                        block.block_number(),
                        block.view_number()
                    );
                    if consensus.is_established() {
                        log::info!("\t- block contained {} tx", tx_count);
                        log::info!("\t- mempool contains {} tx", mempool_count);
                    }

                    tx_count_total += tx_count;

                    let se = StatsExert {
                        time,
                        height: block.header().block_number(),
                        tx_count,
                    };

                    stat_exerts.push_back(se.clone());

                    se
                } else {
                    panic!("blab");
                };

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
                    let av_tx = tx_count_total / rolling_window;

                    let tps = tx_count_total as f32 / diff.as_secs_f32();

                    log::info!("Average over the last {} blocks:", rolling_window);
                    log::info!("\tBlock time: {:?}", av_block_time);
                    log::info!("\tTx per block: {:?}", av_tx);
                    log::info!("\tTx per second: {:?}", tps);

                    tx_count_total -= oldest_block.tx_count;
                }
            }
        }
    }
}

const UNIT_KEY: &str = "6dac424fb81c02e727925d2ad5e8e0506639be6e037799c68b723146ca054faa";
const DEV_KEY: &str = "1ef7aad365c195462ed04c275d47189d5362bbfe36b5e93ce7ba2f3add5f439b";

async fn spam(mempool: std::sync::Arc<Mempool>, consensus: ConsensusProxy, count: usize) {
    let (number, net_id) = {
        let blockchain = consensus.blockchain.read();
        (blockchain.block_number(), blockchain.network_id)
    };
    tokio::task::spawn_blocking(move || {
        let private_key = match net_id {
            NetworkId::UnitAlbatross => UNIT_KEY,
            NetworkId::DevAlbatross => DEV_KEY,
            _ => panic!("Unsupported network"),
        };

        let key_pair = KeyPair::from(PrivateKey::from_str(private_key).unwrap());

        let txs = generate_transactions(&key_pair, number, net_id, count);

        for tx in txs {
            let consensus1 = consensus.clone();
            let mp = std::sync::Arc::clone(&mempool);
            tokio::spawn(async move {
                mp.add_transaction(tx.clone()).await;
                consensus1.send_transaction(tx).await;
            });
        }
    })
    .await;
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
        );
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
