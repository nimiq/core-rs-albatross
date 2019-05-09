use num_bigint::BigUint;
use num_traits::pow;
use parking_lot::RwLock;
#[cfg(feature = "coin")]
use crate::coin::Coin;
use fixed_unsigned::types::FixedUnsigned10;


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

/// Total supply in satoshis.
pub const TOTAL_SUPPLY: u64 = 2_100_000_000_000_000;

/// Initial supply in satoshis.
const INITIAL_SUPPLY: u64 = 252_000_000_000_000;

/// First block using constant tail emission until total supply is reached.
const EMISSION_TAIL_START: u32 = 48_692_960;

/// Constant tail emission in satoshis until total supply is reached.
const EMISSION_TAIL_REWARD: u64 = 4000;

/// Emission speed.
const EMISSION_SPEED: u64 = 4_194_304;

/// Number of active validators
pub const ACTIVE_VALIDATORS: u16 = 512;

/// ceil(2/3) of active validators
// (2 * n + 3) / 3 = ceil(2f + 1) where n = 3f + 1
pub const TWO_THIRD_VALIDATORS: u16 = (2 * ACTIVE_VALIDATORS + 3) / 3;

pub const EPOCH_LENGTH: u32 = 21600;

/// Number of micro blocks to wait for unstaking after next macro block.
pub const UNSTAKE_DELAY: u32 = 100; // TODO: Set.

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

pub fn next_macro_block(block_height: u32) -> u32 {
    // TODO: Implement
    (block_height / 100 + 1) * 100 // Mock
}

#[cfg(feature = "coin")]
pub fn block_reward_at(block_height: u32) -> Coin {
    assert!(block_height >= 1, "block_height must be >= 1");
    let current_supply = supply_after(block_height - 1);
    Coin::from_u64(compute_block_reward(current_supply, block_height)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_correctly_computes_block_reward() {
        assert_eq!(block_reward_at(1), Coin::from_u64(440597534).unwrap());
        assert_eq!(block_reward_at(2), Coin::from_u64(440597429).unwrap());
        assert_eq!(block_reward_at(3), Coin::from_u64(440597324).unwrap());
        assert_eq!(block_reward_at(1000), Coin::from_u64(440492605).unwrap());
        assert_eq!(block_reward_at(4999), Coin::from_u64(440072823).unwrap());
        assert_eq!(block_reward_at(5000), Coin::from_u64(440072718).unwrap());
        assert_eq!(block_reward_at(5001), Coin::from_u64(440072613).unwrap());
        assert_eq!(block_reward_at(5002), Coin::from_u64(440072508).unwrap());
        assert_eq!(block_reward_at(100000), Coin::from_u64(430217207).unwrap());
        assert_eq!(block_reward_at(10000000), Coin::from_u64(40607225).unwrap());
        assert_eq!(block_reward_at(48692959), Coin::from_u64(4001).unwrap());
        assert_eq!(block_reward_at(48692960), Coin::from_u64(4000).unwrap());
        assert_eq!(block_reward_at(52888984), Coin::from_u64(4000).unwrap());
        assert_eq!(block_reward_at(52888985), Coin::from_u64(0).unwrap());
    }

    #[test]
    fn it_correctly_computes_supply() {
        assert_eq!(supply_after(0), 252000000000000);
        assert_eq!(supply_after(1), 252000440597534);
        assert_eq!(supply_after(5000), 254201675369298);
        assert_eq!(supply_after(52888983), 2099999999996000);
        assert_eq!(supply_after(52888984), 2100000000000000);
    }
}
