use std::{
    collections::VecDeque,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock},
};

use clap::Parser;
use futures::StreamExt;
use log::info;
use nimiq::client::ConsensusProxy;
pub use nimiq::{
    client::{Client, Consensus},
    config::{command_line::CommandLine, config::ClientConfig, config_file::ConfigFile},
    error::Error,
    extras::{
        deadlock::initialize_deadlock_detection,
        logging::{initialize_logging, log_error_cause_chain},
        panic::initialize_panic_reporting,
        signal_handling::initialize_signal_handler,
    },
};
use nimiq_block::BlockType;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_keys::{Address, KeyPair, PrivateKey, SecureGenerate};
use nimiq_mempool::mempool::Mempool;
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;
use rand::{
    distributions::{Distribution, WeightedIndex},
    thread_rng, Rng,
};
use serde::Deserialize;

#[derive(Debug, Parser)]
pub struct SpammerCommandLine {
    /// Use a custom configuration file.
    ///
    /// # Examples
    ///
    /// * `nimiq-spammer --config ~/.nimiq/client-albatross.toml`
    #[clap(long, short)]
    pub config: Option<PathBuf>,

    /// Transactions per block to generate.
    ///
    /// * `nimiq-spammer --tpb 724`
    #[clap(long, short)]
    pub tpb: Option<u32>,

    /// A spammer generation config file
    ///
    /// # Examples
    ///
    /// * `nimiq-spammer --profile spammer-profile.toml`
    ///
    #[clap(long, short)]
    pub profile: Option<PathBuf>,
}

pub struct SpammerAccounts {
    // KeyPair associated with the account
    key_pair: KeyPair,
    // Current balance for that account
    balance: Coin,
    // The block number where the txn was sent
    block_number: u32,
}

pub struct SpammerContracts {
    // KeyPair associated with the contract owner
    key_pair: KeyPair,
    // The block number where the contract was created
    block_number: u32,
    // Contract address
    address: Address,
}

pub struct SpammerState {
    balances: Vec<SpammerAccounts>,
    current_block_number: u32,
    vesting_contracs: Vec<SpammerContracts>,
}

#[derive(Deserialize)]
pub struct SpammerGenerationOptions {
    // Weights between transaction types
    // base basic, burst basic, vesting
    weights: [u32; 3],
    // Existing account to new account proportion
    many_to_many: f32,
    // transactions per block, has different meaning depending on burst option
    tpb: usize,
}

impl Default for SpammerGenerationOptions {
    fn default() -> Self {
        Self {
            //By default, only base basic transactions
            weights: [10, 0, 0],
            //By default, only "one to many" distribution
            many_to_many: 0.0,
            //Default constant TPB
            tpb: 500,
        }
    }
}

#[derive(Clone)]
struct StatsExert {
    pub time: std::time::Duration,
    pub is_micro: bool,
    pub tx_count: usize,
}

enum SpamType {
    BaseBasicTransaction,
    BurstBasicTransaction,
    Vesting,
}

const UNIT_KEY: &str = "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";
const DEV_KEY: &str = "1ef7aad365c195462ed04c275d47189d5362bbfe36b5e93ce7ba2f3add5f439b";

async fn main_inner() -> Result<(), Error> {
    // Keep for potential future reactivation
    // initialize_deadlock_detection();

    // Parse command line.
    let spammer_command_line = SpammerCommandLine::parse();
    log::trace!("Command line: {:#?}", spammer_command_line);

    let command_line = CommandLine {
        config: spammer_command_line.config,
        log_level: None,
        log_tags: None,
        passive: false,
        sync_mode: None,
        network: None,
        prove: false,
    };

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    log::trace!("Config file: {:#?}", config_file);

    let state = std::sync::Arc::new(RwLock::new(SpammerState {
        balances: Vec::new(),
        current_block_number: 0,
        vesting_contracs: Vec::new(),
    }));

    // Initialize logging with config values.
    initialize_logging(Some(&command_line), Some(&config_file.log))?;

    // Initialize panic hook.
    initialize_panic_reporting();

    // Initialize signal handler
    initialize_signal_handler();

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
    let metrics_config = config.metrics_server.clone();

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
    let mut client: Client = Client::from_config(
        config,
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
    )
    .await?;
    log::info!("Client initialized");

    // Initialize RPC server
    if let Some(rpc_config) = rpc_config {
        use nimiq::extras::rpc_server::initialize_rpc_server;
        let rpc_server = initialize_rpc_server(&client, rpc_config, client.wallet_store())
            .expect("Failed to initialize RPC server");
        tokio::spawn(async move { rpc_server.run().await });
    }

    // Start consensus.
    let consensus = client.take_consensus().unwrap();

    let mut bc_events = {
        let bc = consensus.blockchain.read();
        bc.notifier_as_stream()
    };

    log::info!("Spawning consensus");
    tokio::spawn(consensus);
    let consensus = client.consensus_proxy();

    // Initialize metrics server
    if let Some(metrics_config) = metrics_config {
        use nimiq::extras::metrics_server::start_metrics_server;
        start_metrics_server(
            metrics_config.addr,
            client.blockchain(),
            client.mempool(),
            client.consensus_proxy(),
            client.network(),
            &[],
        )
    }

    // Start Spammer
    let mempool = if let Some(validator) = client.take_validator() {
        log::info!("Spawning spammer");
        let mempool = std::sync::Arc::clone(&validator.mempool);
        tokio::spawn(validator);
        mempool
    } else {
        panic!("Could not start spammer");
    };

    let rolling_window = Policy::blocks_per_batch() as usize;

    let mut stat_exerts: VecDeque<StatsExert> = VecDeque::new();
    let mut tx_count_total = 0usize;
    let mut micro_block_count = 0usize;

    let mut conf_options = if let Some(spammer_profile) = spammer_command_line.profile {
        let content = std::fs::read_to_string(spammer_profile)?;

        let config: SpammerGenerationOptions = toml::from_str(&content).unwrap();
        config
    } else {
        SpammerGenerationOptions::default()
    };

    //Command line option takes precedence over config file
    if let Some(tpb) = spammer_command_line.tpb {
        conf_options.tpb = tpb as usize;
    }

    let conf_options = Arc::new(conf_options);

    log::info!(
        "Spammer configured to generate {} tx/block",
        conf_options.tpb
    );

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
                        .get_block(&hash, true)
                        .expect("Failed to get latest block")
                };

                log::info!("\n");
                if consensus.is_established() {
                    spam(
                        std::sync::Arc::clone(&mempool),
                        consensus.clone(),
                        key_pair.clone(),
                        conf_options.clone(),
                        std::sync::Arc::clone(&state),
                    )
                    .await;
                }

                let time = std::time::Duration::from_millis(block.header().timestamp());
                let tx_count = block.transactions().map(|txs| txs.len()).unwrap_or(0);
                let mempool_count = mempool.num_transactions();

                info!(
                    block_number = block.block_number(),
                    "Blockchain extended to #{}",
                    block.block_number(),
                );

                if consensus.is_established() {
                    info!(
                        tx_count = tx_count,
                        mempool_count = mempool_count,
                        "Transactions statistics"
                    );
                }

                state.write().unwrap().current_block_number = block.block_number();

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

                    info!(
                        blocks_window = rolling_window,
                        block_time = av_block_time.as_secs_f32(),
                        tpb = av_tx,
                        tps = tps,
                        "Statistics collected"
                    );

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
    config: Arc<SpammerGenerationOptions>,
    state: Arc<RwLock<SpammerState>>,
) {
    let (number, net_id) = {
        let blockchain = consensus.blockchain.read();
        (blockchain.block_number(), blockchain.network_id())
    };
    tokio::task::spawn_blocking(move || {
        let choices = [
            SpamType::BaseBasicTransaction,
            SpamType::BurstBasicTransaction,
            SpamType::Vesting,
        ];

        let dist = WeightedIndex::new(config.weights).unwrap();
        let mut rng = thread_rng();
        let new_count;

        let txs = match choices[dist.sample(&mut rng)] {
            SpamType::BaseBasicTransaction => {
                new_count = config.tpb;
                generate_basic_transactions(
                    &key_pair,
                    number,
                    net_id,
                    new_count,
                    config.clone(),
                    state,
                )
            }
            SpamType::BurstBasicTransaction => {
                new_count = rng.gen_range(config.tpb * 10..config.tpb * 20);
                generate_basic_transactions(
                    &key_pair,
                    number,
                    net_id,
                    new_count,
                    config.clone(),
                    state,
                )
            }
            SpamType::Vesting => {
                new_count = rng.gen_range(0..config.tpb);
                generate_vesting_contracts(&key_pair, number, net_id, new_count, state)
            }
        };

        for tx in txs {
            let consensus1 = consensus.clone();
            let mp = std::sync::Arc::clone(&mempool);
            tokio::spawn(async move {
                if let Err(e) = mp.add_transaction(tx.clone(), None).await {
                    log::warn!("Mempool rejected transaction: {:?} - {:#?}", e, tx);
                }
                if let Err(e) = consensus1.send_transaction(tx).await {
                    log::warn!("Failed to send transaction: {:?}", e);
                }
            });
        }
        log::info!("\tSent {} transactions to the network.\n", new_count);
    })
    .await
    .expect("spawn_blocking() panicked");
}

fn generate_basic_transactions(
    key_pair: &KeyPair,
    start_height: u32,
    network_id: NetworkId,
    count: usize,
    config: Arc<SpammerGenerationOptions>,
    state: std::sync::Arc<RwLock<SpammerState>>,
) -> Vec<Transaction> {
    let mut txs = Vec::new();

    let mut rng = thread_rng();

    for _ in 0..count {
        let current_block_number = state.read().unwrap().current_block_number;
        let mut state = state.write().unwrap();

        if rng.gen_bool(config.many_to_many.into()) && !state.balances.is_empty() {
            //This is the case where we send from an existing account

            // Obtain a random index
            let index = rng.gen_range(0..state.balances.len());

            let account = &mut state.balances[index];

            // If the sender already reached a balance of zero we need to remove it
            if account.balance == Coin::ZERO {
                state.balances.swap_remove(index);
                continue;
            }

            //We need to make sure the txns are mined and included in the blockchain first.
            if current_block_number - account.block_number < Policy::blocks_per_batch() {
                continue;
            }

            // We generate a new recipient
            let new_kp = KeyPair::generate(&mut rng);
            let recipient = Address::from(&new_kp);
            let amount = Coin::from_u64_unchecked(1);

            let tx = TransactionBuilder::new_basic(
                &account.key_pair,
                recipient,
                amount,
                Coin::ZERO,
                start_height,
                network_id,
            )
            .unwrap();
            txs.push(tx);

            //Update the senders balance
            account.balance -= amount;
            //Create a new recipients account and add it to the vector
            state.balances.push(SpammerAccounts {
                key_pair: new_kp,
                balance: amount,
                block_number: current_block_number,
            });
            continue;
        }

        // This is the case where we are creating new recipient accounts
        let new_kp = KeyPair::generate(&mut rng);

        let recipient = Address::from(&new_kp);
        let amount = Coin::from_u64_unchecked(100);

        //We only need to mantain state when we use many to many distributions
        if config.many_to_many > 0.0 {
            state.balances.push(SpammerAccounts {
                key_pair: new_kp,
                balance: amount,
                block_number: current_block_number,
            });
        }

        let tx = TransactionBuilder::new_basic(
            key_pair,
            recipient,
            amount,
            Coin::ZERO,
            start_height,
            network_id,
        )
        .unwrap();
        txs.push(tx);
    }

    txs
}

fn generate_vesting_contracts(
    key_pair: &KeyPair,
    start_height: u32,
    network_id: NetworkId,
    count: usize,
    state: Arc<RwLock<SpammerState>>,
) -> Vec<Transaction> {
    let mut txs = Vec::new();

    let mut rng = thread_rng();

    let mut state = state.write().unwrap();
    let current_block_number = state.current_block_number;

    state.vesting_contracs.retain(|contract| {
        if current_block_number - contract.block_number < Policy::blocks_per_batch() {
            true
        } else {
            let tx = TransactionBuilder::new_redeem_vesting(
                &contract.key_pair,
                contract.address.clone(),
                Address::from(&contract.key_pair),
                Coin::from_u64_unchecked(10),
                Coin::ZERO,
                start_height,
                network_id,
            )
            .unwrap();
            txs.push(tx);
            false
        }
    });

    for _ in 0..count {
        let new_kp = KeyPair::generate(&mut rng);
        let recipient = Address::from(&new_kp);

        let tx = TransactionBuilder::new_create_vesting(
            key_pair,
            recipient,
            1,
            1,
            1,
            Coin::from_u64_unchecked(10),
            Coin::ZERO,
            start_height,
            network_id,
        )
        .unwrap();

        state.vesting_contracs.push(SpammerContracts {
            block_number: current_block_number,
            key_pair: new_kp,
            address: tx.recipient.clone(),
        });

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
