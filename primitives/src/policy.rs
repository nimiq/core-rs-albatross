use bigdecimal::BigDecimal;
use num_bigint::BigUint;
use num_traits::pow;
use parking_lot::RwLock;
#[cfg(feature = "coin")]
use crate::coin::Coin;
use fixed_unsigned::types::FixedUnsigned10;

/// The highest (easiest) block PoW target.
lazy_static! {
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
const TOTAL_SUPPLY: u64 = 2100000000000000;

/// Initial supply in satoshis.
const INITIAL_SUPPLY: u64 = 252000000000000;

/// First block using constant tail emission until total supply is reached.
const EMISSION_TAIL_START: u32 = 48692960;

/// Constant tail emission in satoshis until total supply is reached.
const EMISSION_TAIL_REWARD: u64 = 4000;

/// Emission speed.
const EMISSION_SPEED: u64 = 4194304;

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
    return supply_between(supply, end_i * SUPPLY_CACHE_INTERVAL, block_height + 1);
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
    Coin::from(compute_block_reward(current_supply, block_height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_correctly_computes_block_reward() {
        assert_eq!(block_reward_at(1), 440597534.into());
        assert_eq!(block_reward_at(2), 440597429.into());
        assert_eq!(block_reward_at(3), 440597324.into());
        assert_eq!(block_reward_at(1000), 440492605.into());
        assert_eq!(block_reward_at(4999), 440072823.into());
        assert_eq!(block_reward_at(5000), 440072718.into());
        assert_eq!(block_reward_at(5001), 440072613.into());
        assert_eq!(block_reward_at(5002), 440072508.into());
        assert_eq!(block_reward_at(100000), 430217207.into());
        assert_eq!(block_reward_at(10000000), 40607225.into());
        assert_eq!(block_reward_at(48692959), 4001.into());
        assert_eq!(block_reward_at(48692960), 4000.into());
        assert_eq!(block_reward_at(52888984), 4000.into());
        assert_eq!(block_reward_at(52888985), 0.into());
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
