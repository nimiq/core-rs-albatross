use block::MacroHeader;
use primitives::coin::Coin;
use primitives::policy;
use std::convert::TryInto;

/// Parses the genesis supply and timestamp from the genesis block. We require both values to
/// calculate the block rewards.
pub fn genesis_parameters(genesis_block: &MacroHeader) -> (Coin, u64) {
    assert_eq!(genesis_block.block_number, 0);

    let extra_data = &genesis_block.extra_data;

    let supply;

    // Try reading supply from genesis block.
    if extra_data.len() < 8 {
        warn!("Genesis block does not encode initial supply, assuming zero.");
        supply = Coin::ZERO;
    } else {
        let bytes = extra_data[..8].try_into().expect("slice has wrong size");
        supply = Coin::from_u64_unchecked(u64::from_be_bytes(bytes));
    }

    (supply, genesis_block.timestamp)
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

    let prev_supply = policy::supply_at(genesis_supply_u64, genesis_timestamp, previous_timestamp);

    let current_supply =
        policy::supply_at(genesis_supply_u64, genesis_timestamp, current_timestamp);

    Coin::from_u64_unchecked(current_supply - prev_supply)
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
