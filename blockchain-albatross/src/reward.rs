use block::MacroBlock;
use primitives::coin::Coin;
use primitives::policy;
use std::convert::TryInto;

/// Parses the genesis supply and timestamp from the genesis block.
pub fn genesis_parameters(genesis_block: &MacroBlock) -> (Coin, u64) {
    assert_eq!(genesis_block.header.block_number, 0);

    let extra_data = &genesis_block.header.extra_data;

    let supply;
    // Try reading supply from genesis block.
    if extra_data.len() < 8 {
        warn!("Genesis block does not encode initial supply, assuming zero.");
        supply = Coin::ZERO;
    } else {
        let bytes = extra_data[..8].try_into().expect("slice has wrong size");
        supply = Coin::from_u64_unchecked(u64::from_be_bytes(bytes));
    }

    (supply, genesis_block.header.timestamp)
}

/// Compute the block reward for a batch from the current macro block, the previous macro block,
/// and the genesis block.
/// This does not include the reward from transaction fees.
pub fn block_reward_for_batch_with_genesis(
    current_block: &MacroBlock,
    previous_macro: &MacroBlock,
    genesis_block: &MacroBlock,
) -> Coin {
    let (supply, timestamp) = genesis_parameters(genesis_block);
    block_reward_for_batch(current_block, previous_macro, supply, timestamp)
}

/// Compute the block reward for an batch from the current macro block, the previous macro block,
/// and the genesis parameters.
/// This does not include the reward from transaction fees.
pub fn block_reward_for_batch(
    current_block: &MacroBlock,
    previous_macro: &MacroBlock,
    genesis_supply: Coin,
    genesis_timestamp: u64,
) -> Coin {
    let genesis_supply_u64 = u64::from(genesis_supply);
    let prev_supply = Coin::from_u64_unchecked(policy::supply_at(
        genesis_supply_u64,
        genesis_timestamp,
        previous_macro.header.timestamp,
    ));
    let current_supply = Coin::from_u64_unchecked(policy::supply_at(
        genesis_supply_u64,
        genesis_timestamp,
        current_block.header.timestamp,
    ));
    current_supply - prev_supply
}
