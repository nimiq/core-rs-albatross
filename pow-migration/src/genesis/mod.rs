pub mod types;

use std::{fs, path::PathBuf, str::FromStr, time::Instant};

use nimiq_database::DatabaseProxy;
use nimiq_genesis_builder::config::GenesisConfig;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{KeyPair, SecureGenerate};
use nimiq_primitives::networks::NetworkId;
use nimiq_rpc::Client;
use nimiq_vrf::VrfSeed;
use rand::{rngs::StdRng, SeedableRng};
use time::OffsetDateTime;

use crate::{
    genesis::types::{Error, PoSRegisteredAgents, PoWRegistrationWindow},
    history::get_history_root,
    state::{get_accounts, get_stakers, get_validators},
};

// POW estimated block time in milliseconds
const POW_BLOCK_TIME_MS: u64 = 60 * 1000; // 1 min

/// Gets the genesis config file
pub async fn get_pos_genesis(
    client: &Client,
    pow_reg_window: &PoWRegistrationWindow,
    network_id: NetworkId,
    env: DatabaseProxy,
    pos_registered_agents: Option<PoSRegisteredAgents>,
) -> Result<GenesisConfig, Error> {
    let seed_message = match network_id {
        NetworkId::TestAlbatross => Some("Albatross TestNet".to_string()),
        NetworkId::MainAlbatross => Some("Albatross MainNet".to_string()),
        _ => {
            log::error!(%network_id, "Unsupported network ID as a target for the migration process");
            return Err(Error::InvalidNetworkId(network_id));
        }
    };

    // Get block according to arguments and check if it exists
    let final_block = client
        .get_block_by_hash(&pow_reg_window.final_block, false)
        .await
        .map_err(|_| {
            log::error!(
                hash = pow_reg_window.final_block,
                "Could not find provided block"
            );
            Error::UnknownBlock
        })?;
    let pow_genesis = client.get_block_by_number(1, false).await?;

    // Build history tree
    log::info!(
        pow_block_number = final_block.number,
        "Building history tree. This may take some time"
    );
    let start = Instant::now();
    let history_root = match get_history_root(client, final_block.number, env).await {
        Ok(history_root) => {
            let duration = start.elapsed();
            log::info!(
                duration = humantime::format_duration(duration).to_string(),
                history_root = history_root.to_hex(),
                "Finished building history tree"
            );
            history_root
        }
        Err(e) => {
            log::error!(error = ?e, "Failed to build history root");
            std::process::exit(1);
        }
    };

    // The PoS genesis timestamp is the cutting block timestamp plus a custom delay
    let pos_genesis_ts =
        pow_reg_window.confirmations as u64 * POW_BLOCK_TIME_MS + final_block.timestamp as u64;
    // The parent election hash of the PoS genesis is the hash of the PoW genesis block
    let parent_election_hash = Blake2bHash::from_str(&pow_genesis.hash)?;
    // The parent hash of the PoS genesis is the hash of cutting block
    let parent_hash = Blake2bHash::from_str(&final_block.hash)?;

    // Build up the VRF seed using a random seed generator seeded with the final block hash
    let mut parent_hash_bytes = [0u8; 32];
    parent_hash_bytes.copy_from_slice(parent_hash.as_slice());
    let mut rng = StdRng::from_seed(parent_hash_bytes);
    let vrf_seed = VrfSeed::default().sign_next_with_rng(&KeyPair::generate(&mut rng), &mut rng);

    log::info!("Getting PoW account state");
    let genesis_accounts = get_accounts(client, &final_block, pos_genesis_ts).await?;

    let (genesis_stakers, genesis_validators) =
        if let Some(registered_agents) = pos_registered_agents {
            (registered_agents.stakers, registered_agents.validators)
        } else {
            log::info!("Getting registered validators in the PoW chain");
            let genesis_validators = get_validators(
                client,
                pow_reg_window.validator_start..pow_reg_window.pre_stake_start,
            )
            .await?;

            log::info!("Getting registered stakers in the PoW chain");
            get_stakers(
                client,
                &genesis_validators,
                pow_reg_window.pre_stake_start..pow_reg_window.pre_stake_end,
            )
            .await?
        };

    Ok(GenesisConfig {
        seed_message,
        vrf_seed: Some(vrf_seed),
        parent_election_hash: Some(parent_election_hash),
        parent_hash: Some(parent_hash),
        history_root: Some(history_root),
        block_number: final_block.number,
        timestamp: Some(OffsetDateTime::from_unix_timestamp(pos_genesis_ts as i64)?),
        validators: genesis_validators
            .into_iter()
            .map(|validator| validator.validator)
            .collect(),
        stakers: genesis_stakers,
        basic_accounts: genesis_accounts.basic_accounts,
        vesting_accounts: genesis_accounts.vesting_accounts,
        htlc_accounts: genesis_accounts.htlc_accounts,
    })
}

/// Write the genesis config file to a TOML file
pub fn write_pos_genesis(file_path: &PathBuf, genesis_config: GenesisConfig) -> Result<(), Error> {
    Ok(fs::write(file_path, toml::to_string(&genesis_config)?)?)
}
