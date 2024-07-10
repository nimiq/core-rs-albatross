use std::convert::TryInto;

use nimiq_block::MacroHeader;
use nimiq_primitives::{coin::Coin, policy::Policy};

/// Parses the genesis supply and timestamp from the genesis block. We require both values to
/// calculate the block rewards.
pub fn genesis_parameters(genesis_block: &MacroHeader) -> (Coin, u64) {
    let extra_data = &genesis_block.extra_data;

    // Try reading supply from genesis block.
    let supply = if extra_data.len() < 8 {
        warn!("Genesis block does not encode initial supply, assuming zero.");
        Coin::ZERO
    } else {
        let bytes = extra_data[..8].try_into().expect("slice has wrong size");
        Coin::from_u64_unchecked(u64::from_be_bytes(bytes))
    };

    (supply, genesis_block.timestamp)
}

// Compute the current batch delay(in ms)
pub fn batch_delay(previous_timestamp: u64, current_timestamp: u64) -> u64 {
    let target_ts =
        previous_timestamp + Policy::BLOCK_SEPARATION_TIME * (Policy::blocks_per_batch() as u64);

    current_timestamp.saturating_sub(target_ts)
}

/// Compute the block reward for a batch from the current macro block, the previous macro block,
/// and the genesis parameters.
/// This does not include the reward from transaction fees.
pub fn block_reward_for_batch(
    current_block: &MacroHeader,
    previous_macro: &MacroHeader,
    genesis_supply: Coin,
    genesis_timestamp: u64,
) -> Coin {
    let current_timestamp = current_block.timestamp;

    let previous_timestamp = previous_macro.timestamp;

    assert!(current_timestamp >= previous_timestamp);
    assert!(previous_timestamp >= genesis_timestamp);

    let genesis_supply_u64 = u64::from(genesis_supply);

    let prev_supply = Policy::supply_at(genesis_supply_u64, genesis_timestamp, previous_timestamp);

    let current_supply =
        Policy::supply_at(genesis_supply_u64, genesis_timestamp, current_timestamp);

    // This is the maximum rewards that we can pay for this batch
    let max_rewards = current_supply.saturating_sub(prev_supply);

    // However, there is a penalty if the batch was not produced in time...
    // First we calculate the delay producing the batch:
    let batch_delay = batch_delay(previous_timestamp, current_timestamp);

    let batch_delay_penalty = Policy::batch_delay_penalty(batch_delay);

    debug!(
        block_number = current_block.block_number,
        delay = batch_delay,
        penalty = batch_delay_penalty,
        "Computed the batch delay and penalty (if any)",
    );

    // The final rewards that are given are a percentage based on the penalty (if any) for not producing blocks in time.
    // i.e.: batch_delay_penalty returns a number in the range [0, 1]
    Coin::from_u64_unchecked((max_rewards as f64 * batch_delay_penalty) as u64)
}

/// Compute the block reward for a batch from the current macro block, the previous macro block,
/// and the genesis block.
/// This does not include the reward from transaction fees.
pub fn block_reward_for_batch_with_genesis(
    current_block: &MacroHeader,
    previous_macro: &MacroHeader,
    genesis_block: &MacroHeader,
) -> Coin {
    let (supply, timestamp) = genesis_parameters(genesis_block);

    block_reward_for_batch(current_block, previous_macro, supply, timestamp)
}
