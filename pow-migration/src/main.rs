use std::{fs, process::exit, time::Duration};

use clap::{Parser, Subcommand};
use convert_case::{Case, Casing};
use log::{info, level_filters::LevelFilter};
use nimiq_keys::Address;
use nimiq_lib::config::{config::ClientConfig, config_file::ConfigFile};
use nimiq_pow_migration::{
    exit_with_error,
    genesis::write_pos_genesis,
    get_block_windows,
    history::{get_history_store_height, migrate_history},
    launch_pos_client, migrate,
    state::{get_stakers, get_validators},
};
use nimiq_primitives::networks::NetworkId;
use nimiq_rpc::Client;
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt, Layer};
use url::Url;

/// Command line arguments for the binary
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the PoS configuration file
    #[arg(short, long)]
    config: String,
    /// PoW RPC server URL
    #[arg(long)]
    url: String,
    /// Optional PoW RPC server username
    #[arg(short, long)]
    username: Option<String>,
    /// Optional PoW RPC server password
    #[arg(short, long)]
    password: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Outputs a list of pre-stakers, optionally filtered by a specific validator
    ListStakers {
        /// Filters the pre-stakers for this validator
        #[arg(long)]
        validator: Option<String>,
    },
    /// Outputs a list of registered validators
    ListValidators,
}

fn initialize_logging() {
    let filter = Targets::new()
        .with_default(LevelFilter::DEBUG)
        .with_target("hyper", LevelFilter::WARN);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(true)
                .with_filter(filter),
        )
        .init();
}

#[tokio::main]
async fn main() {
    //    1 - Use the monitor library to send ready txn and determine if enough validators are ready
    //    2 - Once enough validators are ready we select the next genesis candidate and wait until that block is mined
    //    3 - When the genesis candidate is mined we start the genesis generation process
    //    4 - Monitor the PoW chain to detect if the genesis candidate is forked
    //    5 - After X confirmations, start the 2.0 client with the generated genesis block
    //    6 - If a fork is detected, go back to step 3 and repeat

    initialize_logging();

    let args = Args::parse();

    let current_exe_dir = std::env::current_exe()
        .map(|mut path| {
            path.pop();
            path
        })
        .unwrap_or_else(|error| {
            exit_with_error(
                error,
                "Could not find full filesystem path of the current running executable",
            )
        });

    let contents = fs::read_to_string(&args.config)
        .unwrap_or_else(|error| exit_with_error(error, "Could not read file"));

    let config_file: ConfigFile = toml::from_str(&contents).unwrap_or_else(|error| {
        log::error!(file = args.config);
        exit_with_error(error, "Unable to read configuration file");
    });

    let config = ClientConfig::builder()
        .config_file(&config_file)
        .map(|config_builder| {
            config_builder.build().unwrap_or_else(|error| {
                log::error!(file = args.config);
                exit_with_error(error, "Error building configuration");
            })
        })
        .unwrap_or_else(|error| {
            log::error!(file = args.config);
            exit_with_error(error, "Error parsing configuration file");
        });

    let url =
        Url::parse(&args.url).unwrap_or_else(|error| exit_with_error(error, "Invalid RPC URL"));

    let pow_client = if args.username.is_some() && args.password.is_some() {
        Client::new_with_credentials(url, args.username.unwrap(), args.password.unwrap())
    } else {
        Client::new(url)
    };

    let block_windows = get_block_windows(config.network_id)
        .unwrap_or_else(|error| exit_with_error(error, "Couldn't get block windows"));

    // Check to see if the client already has consensus
    loop {
        let status = pow_client.consensus().await.unwrap();
        if status.eq("established") {
            info!("Consensus is established");

            break;
        }
        info!(
            current_block_height = pow_client.block_number().await.unwrap(),
            "Consensus has not been established yet.."
        );
        sleep(Duration::from_secs(10)).await;
    }

    let registered_validators = if args.command.is_some() {
        get_validators(
            &pow_client,
            block_windows.registration_start..block_windows.registration_end,
        )
        .await
        .unwrap_or_else(|error| exit_with_error(error, "Couldn't get validators list"))
    } else {
        vec![]
    };

    if let Some(Commands::ListValidators) = args.command {
        println!("Registered validators:");
        for validator in registered_validators {
            println!("{}", validator.validator.validator_address);
        }
    } else if let Some(Commands::ListStakers { validator }) = args.command {
        if pow_client.block_number().await.unwrap()
            < block_windows.pre_stake_end + block_windows.block_confirmations
        {
            log::error!("The pre-staking window is not closed yet, generating the list is not possible at this time.");
            exit(1);
        }

        let pre_stakers = get_stakers(
            &pow_client,
            &registered_validators,
            block_windows.pre_stake_start..block_windows.pre_stake_end,
        )
        .await
        .map(|(pre_stakers, _)| match validator {
            Some(address) => Address::from_any_str(&address)
                .map(|address| {
                    pre_stakers
                        .into_iter()
                        .filter(|pre_staker| pre_staker.delegation == address)
                        .collect()
                })
                .unwrap_or_else(|error| {
                    log::error!(%address);
                    exit_with_error(
                        error,
                        "Invalid address provided as argument ('--validator, -v')",
                    );
                }),
            None => pre_stakers,
        })
        .unwrap_or_else(|error| exit_with_error(error, "Couldn't get pre-stakers list"));

        println!("Pre-stakers:");
        for pre_staker in pre_stakers {
            println!(
                "{} has delegated stake to {} with a prestake of {} NIM",
                pre_staker.staker_address, pre_staker.delegation, pre_staker.balance,
            );
        }
    } else {
        let validator_address = if let Some(validator_settings) = config.validator {
            info!(
                validator_address = %validator_settings.validator_address,
                "This is our validator address"
            );
            Some(validator_settings.validator_address)
        } else {
            log::warn!(
                "Missing validator section in the configuration file. Running in 'viewer' mode."
            );
            None
        };

        // Create DB environment
        let env = config
            .storage
            .database(
                config.network_id,
                config.consensus.sync_mode,
                config.database,
            )
            .unwrap_or_else(|error| exit_with_error(error, "Unable to create DB environment"));

        // Check that we are doing the migration for a supported network ID and set the genesis environment variable name
        let genesis_env_var_name = match config.network_id {
            NetworkId::TestAlbatross => "NIMIQ_OVERRIDE_TESTNET_CONFIG",
            NetworkId::MainAlbatross => "NIMIQ_OVERRIDE_MAINET_CONFIG",
            _ => {
                log::error!(%config.network_id, "Unsupported network ID as a target for the migration process");
                exit(1);
            }
        };

        // Create channels in order to communicate with the PoW-to-PoS history migrator
        let (tx_candidate_block, rx_candidate_block) = mpsc::channel(16);
        let (tx_migration_completed, rx_migration_completed) =
            watch::channel(get_history_store_height(env.clone()).await);

        // Spawn PoW-to-PoS migrator as seperate task
        tokio::spawn(migrate_history(
            rx_candidate_block,
            tx_migration_completed,
            env.clone(),
            pow_client.clone(),
            block_windows.block_confirmations,
        ));

        // Check that the `nimiq-client` exists
        let pos_client = current_exe_dir.join("nimiq-client");
        if !pos_client.exists() {
            log::error!("Could not find PoS client, run `cargo build [--release]`");
            exit(1);
        };

        // Create directory where the genesis file will be written if it doesn't exist
        let genesis_dir = current_exe_dir.join("genesis");
        if !genesis_dir.exists() {
            fs::create_dir(genesis_dir.clone()).unwrap_or_else(|error| {
                exit_with_error(error, "Could not create genesis directory")
            });
        }
        let genesis_file =
            genesis_dir.join(config.network_id.to_string().to_case(Case::Kebab) + ".toml");

        let mut candidate_block = block_windows.election_candidate;

        // Eagerly instruct to migrate the PoW history up to the first candidate block
        tx_candidate_block
            .send(candidate_block)
            .await
            .unwrap_or_else(|error| {
                exit_with_error(
                    error,
                    "Failed instructing to migrate to the next candidate block",
                )
            });

        // Continue the migration process once the pre-stake window is closed and confirmed
        loop {
            if pow_client.block_number().await.unwrap()
                > block_windows.pre_stake_end + block_windows.block_confirmations
            {
                break;
            }
            sleep(Duration::from_secs(60)).await;
        }

        let genesis_config;

        loop {
            let pos_history_store_height = rx_migration_completed.borrow();
            // Wait for the PoW to PoS history migration to be caught up with the candidate block
            if *pos_history_store_height != candidate_block {
                log::info!(
                    candidate_block,
                    current_migrated_block = *pos_history_store_height,
                    "Waiting for the PoW history to be migrated up until candidate block",
                );

                drop(pos_history_store_height);
                sleep(Duration::from_secs(60)).await;
                continue;
            }

            // Do the migration
            match migrate(
                &pow_client,
                block_windows,
                candidate_block,
                env.clone(),
                &validator_address,
                config.network_id,
            )
            .await
            {
                Ok(obtained_genesis_config) => match obtained_genesis_config {
                    Some(genesis_cfg) => {
                        // We obtained the genesis configuration so we are done
                        genesis_config = genesis_cfg;
                        break;
                    }
                    None => {
                        candidate_block += block_windows.readiness_window;
                        // Instruct to migrate the PoW history up until the next candidate block
                        tx_candidate_block
                            .send(candidate_block)
                            .await
                            .unwrap_or_else(|error| {
                                exit_with_error(
                                    error,
                                    "Failed instructing to migrate to the next candidate block",
                                )
                            });
                        log::info!(
                            new_candidate = candidate_block,
                            "Moving to the next activation window",
                        );
                    }
                },
                Err(error) => {
                    log::error!(?error, "Could not migrate");
                    exit(1);
                }
            }
        }

        // Write the genesis into the FS
        write_pos_genesis(&genesis_file, genesis_config)
            .unwrap_or_else(|error| exit_with_error(error, "Could not write genesis config file"));
        log::info!(
            filename = ?genesis_file,
            "Finished writing PoS genesis to file"
        );

        // Launch PoS client
        launch_pos_client(
            &pos_client,
            &genesis_file,
            &args.config,
            genesis_env_var_name,
        )
        .unwrap_or_else(|error| exit_with_error(error, "Failed to launch POS client"));
    }
}
