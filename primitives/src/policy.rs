use std::cmp;
use std::convert::TryInto;

use lazy_static::lazy_static;
use num_bigint::BigUint;
use num_traits::pow;
use parking_lot::RwLock;

use fixed_unsigned::types::FixedUnsigned10;

#[cfg(feature = "coin")]
use crate::coin::Coin;

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

/// Number of available validator slots. Note that a single validator may own several validator slots.
pub const SLOTS: u16 = 512;

/// Calculates ceil(SLOTS*2/3) which is the minimum number of validators necessary to produce a
/// macro block, a view change and other actions.
/// We use the following formula for the ceiling division:
/// ceil(x/y) = (x+y-1)/y
pub const TWO_THIRD_SLOTS: u16 = (2 * SLOTS + 3 - 1) / 3;

/// Length of a batch including the macro block
pub const BATCH_LENGTH: u32 = 32; // TODO Set

// how many batches constitute an epoch
pub const BATCHES_PER_EPOCH: u32 = 4; // TODO Set

/// Length of epoch including election macro block
pub const EPOCH_LENGTH: u32 = BATCH_LENGTH * BATCHES_PER_EPOCH;

/// Minimum stake for stakers in Lunas (1 NIM = 100,000 Lunas).
/// A staker is someone who delegates their stake to a validator.
pub const MIN_STAKE: u64 = 1;

/// Minimum initial stake for validators in Lunas (1 NIM = 100,000 Lunas).
/// A validator is someone who actually participates in block production. They are akin to miners
/// in proof-of-work.
pub const MIN_VALIDATOR_STAKE: u64 = 100_000_000;



/// Returns the epoch number at a given block number (height).
#[inline]
pub fn epoch_at(block_number: u32) -> u32 {
    (block_number + EPOCH_LENGTH - 1) / EPOCH_LENGTH
}

/// Returns the epoch index at a given block number. The epoch index is the number of a block relative
/// to the the epoch it is in. For example, the first block of any epoch always has an epoch index of 0.
#[inline]
pub fn epoch_index_at(block_number: u32) -> u32 {
    (block_number + EPOCH_LENGTH - 1) % EPOCH_LENGTH
}

/// Returns the batch number at a given `block_number` (height)
#[inline]
pub fn batch_at(block_number: u32) -> u32 {
    (block_number + BATCH_LENGTH - 1) / BATCH_LENGTH
}

/// Returns the batch index at a given block number. The batch index is the number of a block relative
/// to the the batch it is in. For example, the first block of any batch always has an batch index of 0.
#[inline]
pub fn batch_index_at(block_number: u32) -> u32 {
    (block_number + BATCH_LENGTH - 1) % BATCH_LENGTH
}

/// Returns the number (height) of the next election macro block after a given block number (height).
#[inline]
pub fn election_block_after(block_number: u32) -> u32 {
    (block_number / EPOCH_LENGTH + 1) * EPOCH_LENGTH
}

/// Returns the number (height) of the preceding election macro block before a given block number (height).
/// If the given block number is an  election macro block, it returns the election macro block before it.
#[inline]
pub fn election_block_before(block_number: u32) -> u32 {
    if block_number == 0 {
        panic!("Called macro_block_before with block_number 0");
    }
    (block_number - 1) / EPOCH_LENGTH * EPOCH_LENGTH
}

/// Returns the number (height) of the last election macro block at a given block number (height).
/// If the given block number is an election macro block, then it returns that block number.
#[inline]
pub fn last_election_block(block_number: u32) -> u32 {
    block_number / EPOCH_LENGTH * EPOCH_LENGTH
}

/// Returns a boolean expressing if the block at a given block number (height) is an election macro block.
#[inline]
pub fn is_election_block_at(block_number: u32) -> bool {
    epoch_index_at(block_number) == EPOCH_LENGTH - 1
}

/// Returns the number (height) of the next macro block after a given block number (height).
#[inline]
pub fn macro_block_after(block_number: u32) -> u32 {
    (block_number / BATCH_LENGTH + 1) * BATCH_LENGTH
}

/// Returns the number (height) of the preceding macro block before a given block number (height).
/// If the given block number is a macro block, it returns the macro block before it.
#[inline]
pub fn macro_block_before(block_number: u32) -> u32 {
    if block_number == 0 {
        panic!("Called macro_block_before with block_number 0");
    }
    (block_number - 1) / BATCH_LENGTH * BATCH_LENGTH
}

/// Returns the number (height) of the last macro block at a given block number (height).
/// If the given block number is a macro block, then it returns that block number.
#[inline]
pub fn last_macro_block(block_number: u32) -> u32 {
    block_number / BATCH_LENGTH * BATCH_LENGTH
}

/// Returns a boolean expressing if the block at a given block number (height) is a macro block.
#[inline]
pub fn is_macro_block_at(block_number: u32) -> bool {
    batch_index_at(block_number) == BATCH_LENGTH - 1
}

/// Returns a boolean expressing if the block at a given block number (height) is a micro block.
#[inline]
pub fn is_micro_block_at(block_number: u32) -> bool {
    batch_index_at(block_number) != BATCH_LENGTH - 1
}

/// Returns the block number of the first block of the given epoch (which is always a micro block).
pub fn first_block_of(epoch: u32) -> u32 {
    if epoch == 0 {
        panic!("Called first_block_of for epoch 0");
    }
    (epoch - 1) * EPOCH_LENGTH + 1
}

///  Returns the block number of the first block of the given batch (which is always a micro block).
pub fn first_block_of_batch(batch: u32) -> u32 {
    if batch == 0 {
        panic!("Called first_block_of_batch for batch 0");
    }
    (batch - 1) * BATCH_LENGTH + 1
}

/// Returns the block number of the election macro block of the given epoch (which is always the last block).
pub fn election_block_of(epoch: u32) -> u32 {
    epoch * EPOCH_LENGTH
}

/// Returns the block number of the macro block of the given epoch (which is always the last block).
pub fn macro_block_of(batch: u32) -> u32 {
    batch * EPOCH_LENGTH
}

/// First block in reward registry (first block of previous epoch).
/// Returns `0u32` during epoch 0 (genesis) and 1.
pub fn first_block_of_registry(epoch: u32) -> u32 {
    if epoch <= 1 {
        0u32
    } else {
        first_block_of(epoch - 1)
    }
}

/// This is the number of Lunas (1 NIM = 100,000 Lunas) created by second at the genesis of the
/// Nimiq 2.0 chain. The velocity then decreases following the formula:
/// Supply_velocity (t) = Initial_supply_velocity * e^(- Supply_decay * t)
/// Where e is the exponential function and t is the time in seconds since the genesis block.
pub const INITIAL_SUPPLY_VELOCITY: f64 = 875_000.0;

/// The supply decay is a constant that is calculated so that the supply velocity decreases at a
/// steady 1.47% per year.
pub const SUPPLY_DECAY: f64 = 4.692821935e-10;

/// Returns the supply at a given time (as Unix time) in Lunas (1 NIM = 100,000 Lunas). It is
/// calculated using the following formula:
/// Supply (t) = Genesis_supply + Initial_supply_velocity / Supply_decay * (1 - e^(- Supply_decay * t))
/// Where e is the exponential function, t is the time in seconds since the genesis block and
/// Genesis_supply is the supply at the genesis of the Nimiq 2.0 chain.
pub fn supply_at(genesis_supply: u64, genesis_time: u64, current_time: u64) -> u64 {
    assert!(current_time >= genesis_time);

    let t = (current_time - genesis_time) as f64;

    let exponent = -SUPPLY_DECAY * t;

    let supply =
        genesis_supply + (INITIAL_SUPPLY_VELOCITY / SUPPLY_DECAY * (1.0 - exponent.exp())) as u64;

    cmp::min(supply, TOTAL_SUPPLY)
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;

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
    fn it_correctly_computes_batch() {
        assert_eq!(batch_at(0), 0);
        assert_eq!(batch_at(1), 1);
        assert_eq!(batch_at(32), 1);
        assert_eq!(batch_at(33), 2);
    }

    #[test]
    fn it_correctly_computes_batch_index() {
        assert_eq!(batch_index_at(1), 0);
        assert_eq!(batch_index_at(2), 1);
        assert_eq!(batch_index_at(128), 31);
        assert_eq!(batch_index_at(129), 0);
    }

    #[test]
    fn it_correctly_computes_block_positions() {
        assert_eq!(is_macro_block_at(0), true);
        assert_eq!(!is_micro_block_at(0), true);
        assert_eq!(is_election_block_at(0), true);

        assert_eq!(is_macro_block_at(1), false);
        assert_eq!(!is_micro_block_at(1), false);
        assert_eq!(is_election_block_at(1), false);


        assert_eq!(is_macro_block_at(2), false);
        assert_eq!(!is_micro_block_at(2), false);
        assert_eq!(is_election_block_at(2), false);

        assert_eq!(is_macro_block_at(32), true);
        assert_eq!(is_micro_block_at(32), false);
        assert_eq!(is_election_block_at(32), false);

        assert_eq!(is_macro_block_at(127), false);
        assert_eq!(!is_micro_block_at(127), false);
        assert_eq!(is_election_block_at(127), false);

        assert_eq!(is_macro_block_at(128), true);
        assert_eq!(!is_micro_block_at(128), true);
        assert_eq!(is_election_block_at(128), true);

        assert_eq!(is_macro_block_at(129), false);
        assert_eq!(!is_micro_block_at(129), false);
        assert_eq!(is_election_block_at(129), false);

        assert_eq!(is_macro_block_at(160), true);
        assert_eq!(is_micro_block_at(160), false);
        assert_eq!(is_election_block_at(160), false);
    }

    #[test]
    fn it_correctly_computes_macro_numbers() {
        assert_eq!(macro_block_after(0), 32);
        assert_eq!(macro_block_after(1), 32);
        assert_eq!(macro_block_after(127), 128);
        assert_eq!(macro_block_after(128), 160);
        assert_eq!(macro_block_after(129), 160);

        assert_eq!(macro_block_before(1), 0);
        assert_eq!(macro_block_before(2), 0);
        assert_eq!(macro_block_before(127), 96);
        assert_eq!(macro_block_before(128), 96);
        assert_eq!(macro_block_before(129), 128);
        assert_eq!(macro_block_before(130), 128);
    }

    #[test]
    fn it_correctly_computes_election_numbers() {
        assert_eq!(election_block_after(0), 128);
        assert_eq!(election_block_after(1), 128);
        assert_eq!(election_block_after(127), 128);
        assert_eq!(election_block_after(128), 256);
        assert_eq!(election_block_after(129), 256);

        assert_eq!(election_block_before(1), 0);
        assert_eq!(election_block_before(2), 0);
        assert_eq!(election_block_before(127), 0);
        assert_eq!(election_block_before(128), 0);
        assert_eq!(election_block_before(129), 128);
        assert_eq!(election_block_before(130), 128);

        assert_eq!(last_election_block(0), 0);
        assert_eq!(last_election_block(1), 0);
        assert_eq!(last_election_block(127), 0);
        assert_eq!(last_election_block(128), 128);
        assert_eq!(last_election_block(129), 128);
    }

    #[test]
    fn it_correctly_comutes_first_ofs() {
        assert_eq!(first_block_of(1), 1);
        assert_eq!(first_block_of(2), 129);

        assert_eq!(first_block_of_batch(1), 1);
        assert_eq!(first_block_of_batch(2), 33);
        assert_eq!(first_block_of_batch(3), 65);
        assert_eq!(first_block_of_batch(4), 97);
        assert_eq!(first_block_of_batch(5), 129);
    }
}
