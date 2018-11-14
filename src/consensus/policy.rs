use bigdecimal::BigDecimal;
use crate::consensus::base::primitive::Coin;
use num_bigint::BigInt;
use num_traits::pow;

/// The highest (easiest) block PoW target.
lazy_static! {
    pub static ref BLOCK_TARGET_MAX: BigDecimal = {
        BigDecimal::new(pow(BigInt::from(2), 240), 0)
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

pub fn block_reward_at(block_height: u32) -> Coin {
    // TODO
    return 42.into();
}
