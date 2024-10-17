use std::{fs, process::exit, time::Duration};

use clap::{Parser, Subcommand};
use convert_case::{Case, Casing};
use log::info;
use nimiq::{
    config::{config::ClientConfig, config_file::ConfigFile},
    extras::logging::initialize_logging,
};
use nimiq_keys::Address;
use nimiq_pow_migration::{
    async_retryer, exit_with_error,
    genesis::write_pos_genesis,
    get_block_windows,
    history::{get_history_store_height, migrate_history},
    launch_pos_client, migrate, report_online,
    state::{get_stakers, get_validators},
};
use nimiq_primitives::networks::NetworkId;
use nimiq_rpc::Client;
use nimiq_utils::spawn;
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};
use url::Url;

/// We check for online transactions every this amount of blocks
const ONLINE_CHECK_BLOCKS: u32 = 10;

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
    /// Optional additional subcommands
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

#[tokio::main]
async fn main() {
    // 1. Migrate the PoW history up until the first candidate block (or up until
    //    the PoW head minus the required confirmations).
    // 2. Wait for the pre-stake window to close (by waiting the confirmations).
    // 3. Select the next genesis candidate and wait until that block is mined to
    //    generate the corresponding PoS genesis.
    // 4. Wait for enough validators to signal readiness for the genesis candidate
    // 5. If enough validators signal readiness start the 2.0 client with the generated genesis block
    // 6. If not enough validators signal readiness after the activation window span, go to 2.

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

    initialize_logging(None, Some(&config_file.log))
        .unwrap_or_else(|error| exit_with_error(error, "Could not initialize logging"));

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
        let status = async_retryer(|| pow_client.consensus()).await.unwrap();
        if status.eq("established") {
            info!("Consensus is established");

            break;
        }
        info!(
            current_block_height = async_retryer(|| pow_client.block_number()).await.unwrap(),
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
        if async_retryer(|| pow_client.block_number()).await.unwrap()
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

        // Create DB environments
        let pre_genesis_env = config
            .storage
            .pre_genesis_database(config.network_id, config.database)
            .unwrap_or_else(|error| {
                exit_with_error(error, "Unable to create pre-genesis DB environment")
            });

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
        let (tx_migration_completed, rx_migration_completed) = watch::channel(
            get_history_store_height(pre_genesis_env.clone(), config.network_id).await,
        );

        // Spawn PoW-to-PoS migrator as separate task
        spawn(migrate_history(
            rx_candidate_block,
            tx_migration_completed,
            pre_genesis_env.clone(),
            config.network_id,
            pow_client.clone(),
            block_windows.block_confirmations,
            config.consensus.index_history,
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

        let mut latest_online_report = 0;

        // Continue the migration process once the pre-stake window is closed and confirmed
        loop {
            let pow_block_number = async_retryer(|| pow_client.block_number()).await.unwrap();
            let expected_block_number =
                block_windows.pre_stake_end + block_windows.block_confirmations;
            if pow_block_number > expected_block_number {
                break;
            }

            // If we are running as a validator, we report the tool is running to the blockchain
            if let Some(ref validator_address) = validator_address {
                // We check the latest online report every ONLINE_CHECK_BLOCKS blocks
                if pow_block_number > latest_online_report + ONLINE_CHECK_BLOCKS {
                    latest_online_report = report_online(
                        &pow_client,
                        block_windows,
                        pow_block_number,
                        validator_address.clone(),
                    )
                    .await
                    .unwrap_or_else(|error| exit_with_error(error, "Could not report online"));
                }
            }

            log::info!(
                pow_block_number,
                waiting_for = expected_block_number,
                "Waiting for the pre-stake window to close"
            );
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
            let obtained_genesis_config = migrate(
                &pow_client,
                block_windows,
                candidate_block,
                pre_genesis_env.clone(),
                &validator_address,
                config.network_id,
            )
            .await
            .unwrap_or_else(|error| exit_with_error(error, "Could not migrate"));

            if let Some(genesis_cfg) = obtained_genesis_config {
                // We obtained the genesis configuration so we are done
                genesis_config = genesis_cfg;
                break;
            }

            // We didn't obtain the genesis configuration: select the new candidate block
            candidate_block += block_windows.readiness_window;

            // Instruct to migrate the PoW history up until the this next candidate block
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
