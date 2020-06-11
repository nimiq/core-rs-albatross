use std::convert::TryInto;

#[cfg(feature = "coin")]
use crate::coin::Coin;
use fixed_unsigned::types::FixedUnsigned10;
use lazy_static::lazy_static;
use num_bigint::BigUint;
use num_traits::pow;
use parking_lot::RwLock;

lazy_static! {
    /// The highest (easiest) block PoW target.
    pub static ref BLOCK_TARGET_MAX: FixedUnsigned10  = {
        FixedUnsigned10::from(pow(BigUint::from(2u64), 240))
    };
}

/// Targeted block time in seconds.
pub const BLOCK_TIME: u32 = 60;

/// Number of blocks we take into account to calculate next difficulty.
pub const DIFFICULTY_BLOCK_WINDOW: u32 = 120;

/// Limits the rate at which the difficulty is adjusted min/max.
pub const DIFFICULTY_MAX_ADJUSTMENT_FACTOR: f64 = 2f64;

/// Number of blocks a transaction is valid.
pub const TRANSACTION_VALIDITY_WINDOW: u32 = 120;

/// Number of blocks a transaction is valid with Albatross consensus.
pub const TRANSACTION_VALIDITY_WINDOW_ALBATROSS: u32 = 7200;

/// Total supply in units.
pub const TOTAL_SUPPLY: u64 = 2_100_000_000_000_000;

/// Initial supply in units.
const INITIAL_SUPPLY: u64 = 252_000_000_000_000;

/// First block using constant tail emission until total supply is reached.
const EMISSION_TAIL_START: u32 = 48_692_960;

/// Constant tail emission in units until total supply is reached.
const EMISSION_TAIL_REWARD: u64 = 4000;

/// Emission speed.
const EMISSION_SPEED: u64 = 4_194_304;

lazy_static! {
    static ref SUPPLY_CACHE: RwLock<Vec<u64>> = RwLock::new(vec![INITIAL_SUPPLY]);
}

const SUPPLY_CACHE_INTERVAL: u32 = 5000;

fn supply_after(block_height: u32) -> u64 {
    let end_i = block_height / SUPPLY_CACHE_INTERVAL;
    let start_i;
    let mut supply;
    {
        let cache = SUPPLY_CACHE.read();
        start_i = end_i.min(cache.len().saturating_sub(1) as u32);
        supply = cache[start_i as usize];
    }

    // Update cache.
    {
        let mut cache = SUPPLY_CACHE.write();
        for i in start_i..end_i {
            let start_height = i * SUPPLY_CACHE_INTERVAL;
            let end_height = (i + 1) * SUPPLY_CACHE_INTERVAL;
            supply = supply_between(supply, start_height, end_height);
            cache.push(supply);
        }
    }

    // Calculate remaining supply.
    supply_between(supply, end_i * SUPPLY_CACHE_INTERVAL, block_height + 1)
}

fn supply_between(initial_supply: u64, start_height: u32, end_height: u32) -> u64 {
    let mut supply = initial_supply;
    for i in start_height..end_height {
        supply += compute_block_reward(supply, i);
    }
    supply
}

fn compute_block_reward(current_supply: u64, block_height: u32) -> u64 {
    if block_height == 0 {
        return 0;
    }

    let remaining = TOTAL_SUPPLY - current_supply;
    if block_height >= EMISSION_TAIL_START && remaining >= EMISSION_TAIL_REWARD {
        return EMISSION_TAIL_REWARD;
    }

    let remainder = remaining % EMISSION_SPEED;
    (remaining - remainder) / EMISSION_SPEED
}

#[cfg(feature = "coin")]
pub fn block_reward_at(block_height: u32) -> Coin {
    assert!(block_height >= 1, "block_height must be >= 1");
    let current_supply = supply_after(block_height - 1);
    compute_block_reward(current_supply, block_height)
        .try_into()
        .unwrap()
}

/* Albatross */

/// Number of micro blocks to wait for unstaking after next macro block.
pub const UNSTAKING_DELAY: u32 = 100; // TODO: Set.

/// Number of available slots
pub const SLOTS: u16 = 512;

/// Calculates ceil(2 * SLOTS / 3) which is the minimum number of validators necessary to produce a
/// macro block, a view change and other actions.   
/// We use the following formula for the ceiling division:
/// ceil(x/y) = (x+y-1)/y
pub const TWO_THIRD_SLOTS: u16 = (2 * SLOTS + 3 - 1) / 3;

/// Length of epoch including macro block
pub const EPOCH_LENGTH: u32 = 128; // TODO: Set.

/// Minimum stake in units
pub const MIN_STAKE: u64 = 1;

/// Minimum initial_stake for validators in units
pub const MIN_VALIDATOR_STAKE: u64 = 100_000_000;

/// Returns the height of the next macro block after given `block_height`
#[inline]
pub fn macro_block_after(block_number: u32) -> u32 {
    (block_number / EPOCH_LENGTH + 1) * EPOCH_LENGTH
}

/// Returns the height of the preceding macro block before given `block_number`
#[inline]
pub fn macro_block_before(block_number: u32) -> u32 {
    if block_number == 0 {
        panic!("Called macro_block_before with block_number 0");
    }
    (block_number - 1) / EPOCH_LENGTH * EPOCH_LENGTH
}

#[inline]
pub fn epoch_at(block_number: u32) -> u32 {
    (block_number + EPOCH_LENGTH - 1) / EPOCH_LENGTH
}

#[inline]
pub fn epoch_index_at(block_number: u32) -> u32 {
    (block_number + EPOCH_LENGTH - 1) % EPOCH_LENGTH
}

#[inline]
pub fn is_macro_block_at(block_number: u32) -> bool {
    epoch_index_at(block_number) == EPOCH_LENGTH - 1
}

#[inline]
pub fn last_macro_block(block_number: u32) -> u32 {
    block_number / EPOCH_LENGTH * EPOCH_LENGTH
}

#[inline]
pub fn is_micro_block_at(block_height: u32) -> bool {
    !is_macro_block_at(block_height)
}

pub fn successive_micro_blocks(a: u32, b: u32) -> bool {
    a + 1 == b || (a + 2 == b && is_macro_block_at(a + 1))
}

pub fn first_block_of(epoch: u32) -> u32 {
    if epoch == 0 {
        panic!("Called first_block_of for epoch 0");
    }
    epoch * EPOCH_LENGTH - EPOCH_LENGTH + 1
}

/// First block in reward registry (first block of previous epoch)
/// Returns `0u32` during epoch 0 (genesis) and 1.
pub fn first_block_of_registry(epoch: u32) -> u32 {
    if epoch <= 1 {
        0u32
    } else {
        first_block_of(epoch - 1)
    }
}

pub fn macro_block_of(epoch: u32) -> u32 {
    epoch * EPOCH_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn it_correctly_computes_block_reward() {
        assert_eq!(block_reward_at(1), Coin::try_from(440597534).unwrap());
        assert_eq!(block_reward_at(2), Coin::try_from(440597429).unwrap());
        assert_eq!(block_reward_at(3), Coin::try_from(440597324).unwrap());
        assert_eq!(block_reward_at(1000), Coin::try_from(440492605).unwrap());
        assert_eq!(block_reward_at(4999), Coin::try_from(440072823).unwrap());
        assert_eq!(block_reward_at(5000), Coin::try_from(440072718).unwrap());
        assert_eq!(block_reward_at(5001), Coin::try_from(440072613).unwrap());
        assert_eq!(block_reward_at(5002), Coin::try_from(440072508).unwrap());
        assert_eq!(block_reward_at(100000), Coin::try_from(430217207).unwrap());
        assert_eq!(block_reward_at(10000000), Coin::try_from(40607225).unwrap());
        assert_eq!(block_reward_at(48692959), Coin::try_from(4001).unwrap());
        assert_eq!(block_reward_at(48692960), Coin::try_from(4000).unwrap());
        assert_eq!(block_reward_at(52888984), Coin::try_from(4000).unwrap());
        assert_eq!(block_reward_at(52888985), Coin::try_from(0).unwrap());
    }

    #[test]
    fn it_correctly_computes_supply() {
        assert_eq!(supply_after(0), 252000000000000);
        assert_eq!(supply_after(1), 252000440597534);
        assert_eq!(supply_after(5000), 254201675369298);
        assert_eq!(supply_after(52888983), 2099999999996000);
        assert_eq!(supply_after(52888984), 2100000000000000);
    }

    #[test]
    fn it_correctly_computes_epoch() {
        assert_eq!(epoch_at(0), 0);
        assert_eq!(epoch_at(1), 1);
        assert_eq!(epoch_at(128), 1);
        assert_eq!(epoch_at(129), 2);
    }

    #[test]
    fn it_correctly_computes_epoch_index() {
        assert_eq!(epoch_index_at(1), 0);
        assert_eq!(epoch_index_at(2), 1);
        assert_eq!(epoch_index_at(128), 127);
        assert_eq!(epoch_index_at(129), 0);
    }

    #[test]
    fn it_correctly_computes_macro_block_position() {
        assert_eq!(is_macro_block_at(0), true);
        assert_eq!(!is_micro_block_at(0), true);
        assert_eq!(is_macro_block_at(1), false);
        assert_eq!(!is_micro_block_at(1), false);
        assert_eq!(is_macro_block_at(2), false);
        assert_eq!(!is_micro_block_at(2), false);
        assert_eq!(is_macro_block_at(127), false);
        assert_eq!(!is_micro_block_at(127), false);
        assert_eq!(is_macro_block_at(128), true);
        assert_eq!(!is_micro_block_at(128), true);
        assert_eq!(is_macro_block_at(129), false);
        assert_eq!(!is_micro_block_at(129), false);
    }

    #[test]
    fn it_correctly_computes_macro_numbers() {
        assert_eq!(macro_block_after(0), 128);
        assert_eq!(macro_block_after(1), 128);
        assert_eq!(macro_block_after(127), 128);
        assert_eq!(macro_block_after(128), 256);
        assert_eq!(macro_block_after(129), 256);

        assert_eq!(macro_block_before(1), 0);
        assert_eq!(macro_block_before(2), 0);
        assert_eq!(macro_block_before(127), 0);
        assert_eq!(macro_block_before(128), 0);
        assert_eq!(macro_block_before(129), 128);
        assert_eq!(macro_block_before(130), 128);
    }
}
