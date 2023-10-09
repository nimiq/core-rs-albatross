pub mod genesis;
pub mod history;
pub mod monitor;
pub mod state;

use std::{path::PathBuf, process::Command, time::Duration};

use nimiq_database::DatabaseProxy;
use nimiq_genesis_builder::config::GenesisConfig;
use nimiq_keys::Address;
use nimiq_primitives::networks::NetworkId;
use nimiq_rpc::Client;
use thiserror::Error;
use tokio::time::sleep;

use crate::{
    genesis::{
        get_pos_genesis,
        types::{PoSRegisteredAgents, PoWRegistrationWindow},
    },
    monitor::{
        check_validators_ready, generate_ready_tx, get_ready_txns, send_tx,
        types::ValidatorsReadiness,
    },
    state::{get_stakers, get_validators},
};

static TESTNET_BLOCK_WINDOWS: &BlockWindows = &BlockWindows {
    registration_start: 2590000,
    registration_end: 2660000,
    pre_stake_start: 2660000,
    pre_stake_end: 2663100,
    election_candidate: 2664100,
    block_confirmations: 10,
    // This corresponds to ~24 hours.
    readiness_window: 1440,
};

static MAINET_BLOCK_WINDOWS: &BlockWindows = &BlockWindows {
    registration_start: 2590000,
    registration_end: 2660000,
    pre_stake_start: 2660000,
    pre_stake_end: 2663100,
    election_candidate: 2664100,
    block_confirmations: 10,
    // This corresponds to ~24 hours.
    readiness_window: 1440,
};

/// PoW block registration window
///
/// The registration window is a set of blocks in the PoW chain that marks
/// the start and end of different windows as follows:
///
/// ```text
///
///     1              2              3              4              5        6
/// --- | ------------ | ------------ | ------------ | ------------ |------- |
///
/// ```
///
/// 1. Validator registration window start block.
/// 2. Validator registration window end block.
/// 3. Pre-stake registration window start.
/// 4. Pre-stake registration window end block. This block is also the activation
///    window start.
/// 5. The final block in the PoW chain that will be taken as genesis block for the
///    PoS chain. This block must have a block number that can be an election block
///    number in the PoS chain.
/// 6. This is a block whose block number is a number of confirmations away from
///    the final block described in 4.
///
#[derive(Debug)]
pub struct BlockWindows {
    /// Block number of the validator registration window start.
    pub registration_start: u32,
    /// Block number of the validator registration window wnd.
    pub registration_end: u32,
    /// Block number of the validator registration window end which is also
    /// the pre stake registration window start.
    pub pre_stake_start: u32,
    /// Block number of the pre stake registration window end.
    pub pre_stake_end: u32,

    /// The final block from the PoW that is used to create the first PoS election block.
    pub election_candidate: u32,

    /// Number of confirmations after the final block needed for the PoS chain to
    /// start.
    pub block_confirmations: u32,

    /// If not enough validators are ready to start the PoS chain at the election candidate,
    /// a new candidate is elected after readiness_window blocks.
    /// This process is repeated until we start the PoS chain.
    pub readiness_window: u32,
}

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum Error {
    /// Invalid Network ID
    #[error("Invalid Network ID")]
    InvalidNetworkID(NetworkId),
    /// PoS client unexpectedly exited
    #[error("PoS client unexpectedly exited with status: {0}")]
    PoSUnexpectedExit(std::process::ExitStatus),
    /// I/O error
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),
    /// Genesis building error
    #[error("Error building genesis: {0}")]
    Genesis(#[from] crate::genesis::types::Error),
    /// State migration error
    #[error("State migration error: {0}")]
    State(#[from] crate::state::types::Error),
    /// Migration monitor error
    #[error("Migration monitor error: {0}")]
    Monitor(#[from] crate::monitor::types::Error),
    /// History migration error
    #[error("History migration error: {0}")]
    History(#[from] crate::history::Error),
    /// Validator key hasn't been imported
    #[error("Validator key hasn't been imported: {0}")]
    ValidatorKey(Address),
}

/// Get the block windows according to the specified network ID.
pub fn get_block_windows(network_id: NetworkId) -> Result<&'static BlockWindows, Error> {
    match network_id {
        NetworkId::TestAlbatross => Ok(TESTNET_BLOCK_WINDOWS),
        NetworkId::MainAlbatross => Ok(MAINET_BLOCK_WINDOWS),
        _ => Err(Error::InvalidNetworkID(network_id)),
    }
}

/// Performs the PoS migration from PoW by parsing transactions and state of the PoW
/// chain and returning a PoS genesis configuration.
pub async fn migrate(
    pow_client: &Client,
    block_windows: &BlockWindows,
    env: DatabaseProxy,
    validator_address: &Address,
    network_id: NetworkId,
) -> Result<GenesisConfig, Error> {
    // First we obtain the list of registered validators
    let registered_validators = get_validators(
        pow_client,
        block_windows.registration_start..block_windows.registration_end,
    )
    .await?;

    log::debug!("This is the list of registered validators:");

    let mut registered_validator = false;

    for validator in &registered_validators {
        if validator.validator.validator_address == *validator_address {
            registered_validator = true;
        }

        log::debug!(
            validator_address = validator
                .validator
                .validator_address
                .to_user_friendly_address()
        );
    }

    if !registered_validator {
        log::warn!("The validator address that is being used was not registered before!");
        log::warn!("Therefore this validator cannot participate in the readiness voting process");
    } else {
        // If the validator was registered we need to check if the RPC server we are connected to
        // has the account of the validator address in the PoW client wallet.
        // This is necessary to send validator readiness transactions.
        let wallet_addresses = pow_client
            .accounts()
            .await
            .expect("Failed obtaining the list of accounts owned by the RPC server");

        let mut imported_address = false;

        for account in wallet_addresses {
            if let nimiq_rpc::primitives::Account::Basic(basic_account) = account {
                if basic_account.address == validator_address.to_user_friendly_address() {
                    imported_address = true;
                    break;
                }
            }
        }

        if !imported_address {
            log::error!(
                "The validator was registered but its account was not imported into the PoW client"
            );
            return Err(Error::ValidatorKey(validator_address.clone()));
        }
    }

    // Now we obtain the stake distribution
    let (stakers, validators) = get_stakers(
        pow_client,
        &registered_validators,
        block_windows.pre_stake_start..block_windows.pre_stake_end,
    )
    .await?;

    log::debug!("This is the list of stakers:");

    for staker in &stakers {
        log::debug!(
            staker_address = %staker.staker_address,
            balance = %staker.balance
        );
    }

    let mut reported_ready = false;
    let mut next_election_block;
    let mut previous_election_block;
    loop {
        let current_height = pow_client.block_number().await.unwrap();
        log::info!(current_height);

        // We are past the election candidate, so we are now in one of the activation windows
        if current_height > block_windows.election_candidate {
            // First we calculate how many blocks past the candidate we are currently at
            let diff = current_height - block_windows.election_candidate;
            // TODO: We need to iterate all the previous readiness windows to know if the
            // readiness condition was hit before.
            let mul = diff / block_windows.readiness_window;

            previous_election_block =
                block_windows.election_candidate + mul * block_windows.readiness_window;
            next_election_block =
                block_windows.election_candidate + (mul + 1) * block_windows.readiness_window;
        } else {
            next_election_block = block_windows.election_candidate;
            previous_election_block = block_windows.pre_stake_end;
        }

        if !reported_ready && registered_validator {
            // Obtain all the transactions that we have sent previously.
            let transactions = get_ready_txns(
                pow_client,
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

                match send_tx(pow_client, transaction).await {
                    Ok(_) => reported_ready = true,
                    Err(error) => return Err(error.into()),
                }
            } else {
                log::info!("We found a ready transaction from our validator");
                reported_ready = true;
            }
        }

        // Check if we have enough validators ready at this point
        let validators_status = check_validators_ready(
            pow_client,
            validators.clone(),
            previous_election_block..next_election_block,
        )
        .await;
        match validators_status {
            ValidatorsReadiness::NotReady(stake) => {
                log::info!(stake_ready = %stake, "Not enough validators are ready yet",);
            }
            ValidatorsReadiness::Ready(stake) => {
                log::info!(
                    stake_ready = %stake,
                    "Enough validators are ready to start the PoS chain",
                );
                break;
            }
        }

        sleep(Duration::from_secs(60)).await;

        // We need to check if we are still in the same readiness window.
        if next_election_block < pow_client.block_number().await.unwrap() {
            reported_ready = false;
        }
    }

    // Now that we have enough validators ready, we know the exact block that we are going to use
    let candidate = next_election_block;

    log::info!(next_election_candidate = candidate);

    // We wait until the candidate block is mined
    loop {
        if pow_client.block_number().await.unwrap() >= candidate {
            log::info!("We are ready to start the migration process..");
            break;
        } else {
            log::info!(
                election_candidate = candidate,
                current_height = pow_client.block_number().await.unwrap()
            );
            sleep(Duration::from_secs(60)).await;
        }
    }

    let mut previous_hash = "0".to_string();
    let mut genesis_config: Option<GenesisConfig> = None;

    loop {
        // If we have enough confirmations, we can start the 2.0 client
        if pow_client.block_number().await.unwrap() >= candidate + block_windows.block_confirmations
        {
            log::info!("We are ready to start the Nimiq PoS Client..");
            if let Some(genesis_config) = genesis_config {
                return Ok(genesis_config);
            }
        } else {
            // Start the PoS genesis generation process

            // Obtain the genesis candidate block
            let block = pow_client
                .get_block_by_number(candidate, false)
                .await
                .unwrap();

            let current_hash = block.hash.clone();
            log::info!(current_hash = current_hash, "Current genesis hash");

            if previous_hash != current_hash {
                // Start the genesis generation process
                let pow_registration_window = PoWRegistrationWindow {
                    pre_stake_start: block_windows.pre_stake_start,
                    pre_stake_end: block_windows.pre_stake_end,
                    validator_start: block_windows.registration_start,
                    final_block: block.hash,
                    confirmations: block_windows.block_confirmations,
                };

                genesis_config = Some(
                    get_pos_genesis(
                        pow_client,
                        &pow_registration_window,
                        network_id,
                        env.clone(),
                        Some(PoSRegisteredAgents {
                            validators: validators.clone(),
                            stakers: stakers.clone(),
                        }),
                    )
                    .await?,
                );

                // Update block hash
                previous_hash = current_hash;
            } else {
                // Wait for more confirmations
                let current_confirmations = pow_client.block_number().await.unwrap() - candidate;
                log::info!(
                    current_confirmations,
                    "Waiting for more confirmations to start the Nimiq PoS client"
                );
                sleep(Duration::from_secs(60)).await;
            }
        }
    }
}

/// Launches the PoS client using the path to the client, the path to the genesis file,
/// the config file and the name of the environment variable that needs to be exported for
/// properly setting the genesis file.
pub fn launch_pos_client(
    pos_client_path: &PathBuf,
    genesis_file: &PathBuf,
    config_file: &str,
    genesis_env_var_name: &str,
) -> Result<u32, Error> {
    // Start the nimiq PoS client with the generated genesis file
    log::info!(
        filename = ?genesis_file,
        "Launching PoS client with generated genesis"
    );

    // Set the genesis file environment variable
    std::env::set_var(genesis_env_var_name, genesis_file);

    // Launch the PoS client
    let mut child = match Command::new(pos_client_path)
        .arg("-c")
        .arg(config_file)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            log::error!(?error, "Could not launch PoS client");
            return Err(error.into());
        }
    };

    // Check that we were able to launch the PoS client
    match child.try_wait() {
        Ok(Some(status)) => {
            log::error!(%status, "Pos client unexpectedly exited");
            Err(Error::PoSUnexpectedExit(status))
        }
        Ok(None) => {
            let pid = child.id();
            log::info!(pid, "Pos client running");
            Ok(pid)
        }
        Err(error) => {
            log::error!(?error, "Error waiting for the PoS client to run");
            Err(error.into())
        }
    }
}
