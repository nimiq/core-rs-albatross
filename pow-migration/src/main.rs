use std::{fs, process::exit, thread::sleep, time::Duration};

use clap::Parser;
use log::{info, level_filters::LevelFilter};
use nimiq_lib::config::{config::ClientConfig, config_file::ConfigFile};
use nimiq_pow_migration::{
    genesis::{
        get_pos_genesis,
        types::{PoSRegisteredAgents, PoWRegistrationWindow},
        write_pos_genesis,
    },
    get_block_windows,
    monitor::{
        check_validators_ready, generate_ready_tx, get_ready_txns, send_tx,
        types::{ValidatorsReadiness, ACTIVATION_HEIGHT},
    },
    state::{get_stakers, get_validators},
};
use nimiq_primitives::policy::Policy;
use nimiq_rpc::Client;
use serde::Deserialize;
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt, Layer};
use url::Url;

/// Command line arguments for the binary
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the PoS configuration file
    #[arg(short, long)]
    config: String,
    /// Path to the wrapper settings file
    #[arg(short, long)]
    settings: String,
    /// PoS RPC server URL
    #[arg(short, long)]
    url: String,
    /// Optional PoS RPC server username
    #[arg(short, long)]
    username: Option<String>,
    /// Optional PoS RPC server password
    #[arg(short, long)]
    password: Option<String>,
}

// Top level struct to hold the TOML data.
#[derive(Deserialize)]
struct Data {
    genesis: String,
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

    let contents = match fs::read_to_string(&args.config) {
        Ok(c) => c,

        Err(_) => {
            log::error!(file = args.config, "Could not read file");
            exit(1);
        }
    };

    let config_file: ConfigFile = match toml::from_str(&contents) {
        Ok(d) => d,
        Err(error) => {
            log::error!(
                file = args.config,
                ?error,
                "Unable to read configuration file"
            );
            exit(1);
        }
    };

    let config = match ClientConfig::builder().config_file(&config_file) {
        Ok(config) => match config.build() {
            Ok(config) => config,
            Err(error) => {
                log::error!(file = args.config, ?error, "Error building configuration");
                exit(1);
            }
        },
        Err(error) => {
            log::error!(
                file = args.config,
                ?error,
                "Error parsing configuration file"
            );
            exit(1);
        }
    };

    let contents = match fs::read_to_string(&args.settings) {
        Ok(c) => c,

        Err(_) => {
            log::error!(file = args.config, "Could not read file");
            exit(1);
        }
    };

    let settings: Data = match toml::from_str(&contents) {
        Ok(d) => d,
        Err(error) => {
            log::error!(file = args.config, ?error, "Unable to read settings file");
            exit(1);
        }
    };

    let validator_address = if let Some(validator_settings) = config.validator {
        validator_settings.validator_address
    } else {
        log::error!("Missing validator section in the configuration file");
        exit(1);
    };

    info!("This is our validator address: {}", validator_address);

    // Create DB environment
    let env = match config.storage.database(
        config.network_id,
        config.consensus.sync_mode,
        config.database,
    ) {
        Ok(env) => env,
        Err(error) => {
            log::error!(?error, "Unable to create DB environment");
            exit(1);
        }
    };

    let url = match Url::parse(&args.url) {
        Ok(url) => url,
        Err(error) => {
            log::error!(?error, "Invalid RPC URL");
            std::process::exit(1);
        }
    };

    let client = if args.username.is_some() && args.password.is_some() {
        Client::new_with_credentials(url, args.username.unwrap(), args.password.unwrap())
    } else {
        Client::new(url)
    };

    let block_windows = match get_block_windows(config.network_id) {
        Ok(block_windows) => block_windows,
        Err(error) => {
            log::error!(?error, "Couldn't get block windows");
            std::process::exit(1);
        }
    };

    loop {
        let status = client.consensus().await.unwrap();
        if status.eq("established") {
            info!("Consensus is established");

            break;
        }
        info!(
            current_block_height = client.block_number().await.unwrap(),
            "Consensus has not been established yet.."
        );
        sleep(Duration::from_secs(10));
    }

    // This tool is intended to be used past the pre-stake window
    if client.block_number().await.unwrap()
        < block_windows.pre_stake_end + block_windows.block_confirmations
    {
        log::error!("This tool is intended to be used during the activation period");
        exit(1);
    }

    // First we obtain the list of registered validators
    let registered_validators = match get_validators(
        &client,
        block_windows.registration_start..block_windows.registration_end,
    )
    .await
    {
        Ok(validators) => validators,
        Err(error) => {
            log::error!(?error, "Error obtaining the list of registered validators");
            exit(1)
        }
    };

    log::debug!("This is the list of registered validators:");

    for validator in &registered_validators {
        log::debug!(
            validator_address = validator
                .validator
                .validator_address
                .to_user_friendly_address()
        );
    }

    // Now we obtain the stake distribution
    let (stakers, validators) = match get_stakers(
        &client,
        &registered_validators,
        block_windows.pre_stake_start..block_windows.pre_stake_end,
    )
    .await
    {
        Ok((stakers, validators)) => (stakers, validators),
        Err(error) => {
            log::error!(?error, "Error obtaining the list of stakers");
            exit(1)
        }
    };

    log::debug!("This is the list of stakers:");

    for staker in &stakers {
        log::debug!(
            staker_address = %staker.staker_address,
            balance = %staker.balance
        );
    }

    let mut reported_ready = false;
    loop {
        let current_height = client.block_number().await.unwrap();
        info!(current_height);

        let next_election_block = Policy::election_block_after(current_height);
        let mut previous_election_block = Policy::election_block_before(current_height);

        if previous_election_block < ACTIVATION_HEIGHT {
            previous_election_block = ACTIVATION_HEIGHT;
        }

        if !reported_ready {
            // Obtain all the transactions that we have sent previously.
            // TODO: We need to check that this validator is part of the list of the registered validators!
            let transactions = get_ready_txns(
                &client,
                validator_address.to_user_friendly_address(),
                previous_election_block..next_election_block,
            )
            .await;

            if transactions.is_empty() {
                log::info!(
                    previous_election_block,
                    next_election_block,
                    "We didn't find a ready transaction from our validator in this window"
                );
                // Report we are ready to the Nimiq PoW chain:
                let transaction = generate_ready_tx(validator_address.to_user_friendly_address());

                match send_tx(&client, transaction).await {
                    Ok(_) => reported_ready = true,
                    Err(_) => exit(1),
                }
            } else {
                log::info!("We found a ready transaction from our validator");
                reported_ready = true;
            }
        }

        // Check if we have enough validators ready at this point
        let validators_status = check_validators_ready(&client, validators.clone()).await;
        match validators_status {
            ValidatorsReadiness::NotReady(stake) => {
                info!(stake_ready = %stake, "Not enough validators are ready yet",);
            }
            ValidatorsReadiness::Ready(stake) => {
                info!(
                    stake_ready = %stake,
                    "Enough validators are ready to start the PoS chain",
                );
                break;
            }
        }

        sleep(Duration::from_secs(60));

        // If at this point we have a new nex_election_block, it means that we are in a new epoch, so we need to report we are ready again.
        if next_election_block != Policy::election_block_after(client.block_number().await.unwrap())
        {
            reported_ready = false;
        }
    }

    // Now that we have enough validators ready, we need to pick the next election block candidate
    let candidate = Policy::election_block_after(client.block_number().await.unwrap());

    info!(next_election_candidate = candidate);

    loop {
        if client.block_number().await.unwrap() >= candidate + block_windows.block_confirmations {
            info!("We are ready to start the migration process..");
            break;
        } else {
            info!(
                election_candidate = candidate,
                current_height = client.block_number().await.unwrap()
            );
            sleep(Duration::from_secs(60));
        }
    }
    // Obtain the genesis candidate block
    let block = client.get_block_by_number(candidate, false).await.unwrap();

    // Start the genesis generation process
    let pow_registration_window = PoWRegistrationWindow {
        pre_stake_start: block_windows.pre_stake_start,
        pre_stake_end: block_windows.pre_stake_end,
        validator_start: block_windows.registration_start,
        final_block: block.hash,
        confirmations: block_windows.block_confirmations,
    };

    let genesis_config = match get_pos_genesis(
        &client,
        &pow_registration_window,
        env,
        Some(PoSRegisteredAgents {
            validators,
            stakers,
        }),
    )
    .await
    {
        Ok(config) => config,
        Err(error) => {
            log::error!(?error, "Failed to build PoS genesis");
            exit(1);
        }
    };

    if let Err(error) = write_pos_genesis(&settings.genesis, genesis_config) {
        log::error!(?error, "Could not write genesis config file");
        exit(1);
    }
    log::info!(
        filename = settings.genesis,
        "Finished writing PoS genesis to file"
    );
    // Start the nimiq 2.0 client with the generated genesis file
}
