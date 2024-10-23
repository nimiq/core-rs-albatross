use std::{fs, path::PathBuf, str::FromStr, time::Instant};

use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::config::GenesisConfig;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{KeyPair, SecureGenerate};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_rpc::Client;
use nimiq_vrf::VrfSeed;
use rand::{rngs::StdRng, SeedableRng};
use time::OffsetDateTime;

use crate::{
    async_retryer, exit_with_error,
    history::get_history_root,
    state::{get_accounts, POW_BLOCK_TIME},
    types::{BlockWindows, GenesisError, PoSRegisteredAgents},
};

/// Gets the genesis config file
pub async fn get_pos_genesis(
    pow_client: &Client,
    pow_reg_window: &BlockWindows,
    candidate_block: u32,
    network_id: NetworkId,
    env: MdbxDatabase,
    pos_registered_agents: PoSRegisteredAgents,
) -> Result<GenesisConfig, GenesisError> {
    match network_id {
        NetworkId::TestAlbatross => {}
        NetworkId::MainAlbatross => {}
        _ => {
            log::error!(%network_id, "Unsupported network ID as a target for the migration process");
            return Err(GenesisError::InvalidNetworkId(network_id));
        }
    }

    // Get block according to arguments and check if it exists
    let final_block = async_retryer(|| pow_client.get_block_by_number(candidate_block, false))
        .await
        .map_err(|_| {
            log::error!(
                block_number = candidate_block,
                "Could not find provided block"
            );
            GenesisError::UnknownBlock
        })?;
    let pow_genesis = async_retryer(|| pow_client.get_block_by_number(1, false)).await?;

    // Build history tree
    log::info!(
        pow_block_number = final_block.number,
        "Building history tree. This may take some time"
    );
    let start = Instant::now();
    let history_root = get_history_root(env, network_id)
        .await
        .inspect(|history_root| {
            let duration = start.elapsed();
            log::info!(
                duration = humantime::format_duration(duration).to_string(),
                history_root = history_root.to_hex(),
                "Finished building history tree"
            );
        })
        .unwrap_or_else(|error| exit_with_error(error, "Failed to build history root"));

    // The PoS genesis timestamp is the cutting block timestamp plus a custom delay
    let pos_genesis_ts_unix =
        pow_reg_window.block_confirmations as u64 * POW_BLOCK_TIME + final_block.timestamp as u64;
    // The parent election hash of the PoS genesis is the hash of the PoW genesis block
    let parent_election_hash = Blake2bHash::from_str(&pow_genesis.hash)?;
    // The parent hash of the PoS genesis is the hash of cutting block
    let parent_hash = Blake2bHash::from_str(&final_block.hash)?;

    // Build up the VRF seed using a random seed generator seeded with the final block hash
    let mut parent_hash_bytes = [0u8; 32];
    parent_hash_bytes.copy_from_slice(parent_hash.as_slice());
    let mut rng = StdRng::from_seed(parent_hash_bytes);
    let vrf_seed = VrfSeed::default().sign_next_with_rng(
        &KeyPair::generate(&mut rng),
        final_block.number,
        &mut rng,
    );

    log::info!("Getting PoW account state");

    let (genesis_stakers, active_validators, inactive_validators) = (
        pos_registered_agents.stakers,
        pos_registered_agents.active_validators,
        pos_registered_agents.inactive_validators,
    );

    // Calculate how much stake was burnt into registering validators and stakers
    // (the validator's `total_stake` here already includes its staker's delegated balance)
    // We need to consider active + inactive validator
    let burnt_registration_balance = active_validators
        .iter()
        .chain(inactive_validators.iter())
        .fold(Coin::ZERO, |acc, validator| acc + validator.total_stake);

    let genesis_accounts =
        get_accounts(pow_client, &final_block, burnt_registration_balance).await?;

    // Create the genesis validators vectors which include active + inactive.
    // The only difference with the inactive ones, is that they have the inactive from flag set.
    let mut genesis_validators = vec![];
    for validator in inactive_validators {
        let mut val = validator.validator;
        val.inactive_from = Some(candidate_block);
        genesis_validators.push(val);
    }

    for validator in active_validators {
        genesis_validators.push(validator.validator)
    }

    Ok(GenesisConfig {
        network: network_id,
        vrf_seed: Some(vrf_seed),
        parent_election_hash: Some(parent_election_hash),
        parent_hash: Some(parent_hash),
        history_root: Some(history_root),
        block_number: final_block.number,
        timestamp: Some(OffsetDateTime::from_unix_timestamp(
            pos_genesis_ts_unix as i64,
        )?),
        validators: genesis_validators,
        stakers: genesis_stakers,
        basic_accounts: genesis_accounts.basic_accounts,
        vesting_accounts: genesis_accounts.vesting_accounts,
        htlc_accounts: genesis_accounts.htlc_accounts,
    })
}

/// Write the genesis config file to a TOML file
pub fn write_pos_genesis(
    file_path: &PathBuf,
    genesis_config: GenesisConfig,
) -> Result<(), GenesisError> {
    Ok(fs::write(file_path, toml::to_string(&genesis_config)?)?)
}
