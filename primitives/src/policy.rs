use std::cmp;

use nimiq_keys::Address;

/// This is the address for the staking contract. Corresponds to
/// 'NQ38 STAK 1NG0 0000 0000 C0NT RACT 0000 0000'
#[rustfmt::skip]
pub const STAKING_CONTRACT_ADDRESS: Address = Address([
    0xd6, 0xd5, 0x30, 0xda, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x60, 0x2d, 0xbc, 0xa9, 0x9b, 0x00, 0x00, 0x00, 0x00, 0x00,
]);

/// This is the address for the coinbase. Note that this is not a real account, it is just the
/// address we use to denote that some coins originated from a coinbase event. Corresponds to
/// 'NQ81 C01N BASE 0000 0000 0000 0000 0000 0000'
#[rustfmt::skip]
pub const COINBASE_ADDRESS: Address = Address([
    0x60, 0x03, 0x65, 0xab, 0x4e, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
]);

/// Number of blocks a transaction is valid with Albatross consensus.
pub const TRANSACTION_VALIDITY_WINDOW: u32 = 7200;

/// The maximum allowed size, in bytes, for a micro block body.
pub const MAX_SIZE_MICRO_BODY: usize = 100_000;

/// The current version number of the protocol. Changing this always results in a hard fork.
pub const VERSION: u16 = 1;

/// Number of available validator slots. Note that a single validator may own several validator slots.
pub const SLOTS: u16 = 512;

/// Calculates 2f+1 slots which is the minimum number of slots necessary to produce a macro block,
/// a view change and other actions.
/// It is also the minimum number of slots necessary to be guaranteed to have a majority of honest
/// slots. That's because from a total of 3f+1 slots at most f will be malicious. If in a group of
/// 2f+1 slots we have f malicious ones (which is the worst case scenario), that still leaves us
/// with f+1 honest slots. Which is more than the f slots that are not in this group (which must all
/// be honest).
/// It is calculated as `ceil(SLOTS*2/3)` and we use the formula `ceil(x/y) = (x+y-1)/y` for the
/// ceiling division.
pub const TWO_F_PLUS_ONE: u16 = (2 * SLOTS + 3 - 1) / 3;

/// Calculates f+1 slots which is the minimum number of slots necessary to be guaranteed to have at
/// least one honest slots. That's because from a total of 3f+1 slots at most f will be malicious.
/// It is calculated as `ceil(SLOTS/3)` and we use the formula `ceil(x/y) = (x+y-1)/y` for the
/// ceiling division.
pub const F_PLUS_ONE: u16 = (SLOTS + 3 - 1) / 3;

/// Length of a batch including the macro block
pub const BLOCKS_PER_BATCH: u32 = 32; // TODO Set

/// How many batches constitute an epoch
pub const BATCHES_PER_EPOCH: u16 = 4; // TODO Set

/// Length of epoch including election macro block
pub const BLOCKS_PER_EPOCH: u32 = BLOCKS_PER_BATCH * BATCHES_PER_EPOCH as u32;

/// The maximum drift, in milliseconds, that is allowed between any block's timestamp and the node's
/// system time. We only care about drifting to the future.
pub const TIMESTAMP_MAX_DRIFT: u64 = 600000;

/// Tendermint's initial timeout, in milliseconds.
/// See https://arxiv.org/abs/1807.04938v3 for more information.
pub const TENDERMINT_TIMEOUT_INIT: u64 = 1000; // TODO: Set

/// Tendermint's timeout delta, in milliseconds.
/// See https://arxiv.org/abs/1807.04938v3 for more information.
pub const TENDERMINT_TIMEOUT_DELTA: u64 = 1000; // TODO: Set

/// The deposit necessary to create a validator in Lunas (1 NIM = 100,000 Lunas).
/// A validator is someone who actually participates in block production. They are akin to miners
/// in proof-of-work.
pub const VALIDATOR_DEPOSIT: u64 = 1_000_000_000;

/// Total supply in units.
pub const TOTAL_SUPPLY: u64 = 2_100_000_000_000_000;

/// This is the number of Lunas (1 NIM = 100,000 Lunas) created by millisecond at the genesis of the
/// Nimiq 2.0 chain. The velocity then decreases following the formula:
/// Supply_velocity (t) = Initial_supply_velocity * e^(- Supply_decay * t)
/// Where e is the exponential function and t is the time in milliseconds since the genesis block.
pub const INITIAL_SUPPLY_VELOCITY: f64 = 875.0;

/// The supply decay is a constant that is calculated so that the supply velocity decreases at a
/// steady 1.47% per year.
pub const SUPPLY_DECAY: f64 = 4.692821935e-13;

/// Returns the epoch number at a given block number (height).
#[inline]
pub fn epoch_at(block_number: u32) -> u32 {
    (block_number + BLOCKS_PER_EPOCH - 1) / BLOCKS_PER_EPOCH
}

/// Returns the epoch index at a given block number. The epoch index is the number of a block relative
/// to the epoch it is in. For example, the first block of any epoch always has an epoch index of 0.
#[inline]
pub fn epoch_index_at(block_number: u32) -> u32 {
    (block_number + BLOCKS_PER_EPOCH - 1) % BLOCKS_PER_EPOCH
}

/// Returns the batch number at a given `block_number` (height)
#[inline]
pub fn batch_at(block_number: u32) -> u32 {
    (block_number + BLOCKS_PER_BATCH - 1) / BLOCKS_PER_BATCH
}

/// Returns the batch index at a given block number. The batch index is the number of a block relative
/// to the batch it is in. For example, the first block of any batch always has an batch index of 0.
#[inline]
pub fn batch_index_at(block_number: u32) -> u32 {
    (block_number + BLOCKS_PER_BATCH - 1) % BLOCKS_PER_BATCH
}

/// Returns the number (height) of the next election macro block after a given block number (height).
#[inline]
pub fn election_block_after(block_number: u32) -> u32 {
    (block_number / BLOCKS_PER_EPOCH + 1) * BLOCKS_PER_EPOCH
}

/// Returns the block number (height) of the preceding election macro block before a given block number (height).
/// If the given block number is an election macro block, it returns the election macro block before it.
#[inline]
pub fn election_block_before(block_number: u32) -> u32 {
    if block_number == 0 {
        panic!("Called macro_block_before with block_number 0");
    }
    (block_number - 1) / BLOCKS_PER_EPOCH * BLOCKS_PER_EPOCH
}

/// Returns the block number (height) of the last election macro block at a given block number (height).
/// If the given block number is an election macro block, then it returns that block number.
#[inline]
pub fn last_election_block(block_number: u32) -> u32 {
    block_number / BLOCKS_PER_EPOCH * BLOCKS_PER_EPOCH
}

/// Returns a boolean expressing if the block at a given block number (height) is an election macro block.
#[inline]
pub fn is_election_block_at(block_number: u32) -> bool {
    epoch_index_at(block_number) == BLOCKS_PER_EPOCH - 1
}

/// Returns the block number (height) of the next macro block after a given block number (height).
#[inline]
pub fn macro_block_after(block_number: u32) -> u32 {
    (block_number / BLOCKS_PER_BATCH + 1) * BLOCKS_PER_BATCH
}

/// Returns the block number (height) of the preceding macro block before a given block number (height).
/// If the given block number is a macro block, it returns the macro block before it.
#[inline]
pub fn macro_block_before(block_number: u32) -> u32 {
    if block_number == 0 {
        panic!("Called macro_block_before with block_number 0");
    }
    (block_number - 1) / BLOCKS_PER_BATCH * BLOCKS_PER_BATCH
}

/// Returns the block number (height) of the last macro block at a given block number (height).
/// If the given block number is a macro block, then it returns that block number.
#[inline]
pub fn last_macro_block(block_number: u32) -> u32 {
    block_number / BLOCKS_PER_BATCH * BLOCKS_PER_BATCH
}

/// Returns a boolean expressing if the block at a given block number (height) is a macro block.
#[inline]
pub fn is_macro_block_at(block_number: u32) -> bool {
    batch_index_at(block_number) == BLOCKS_PER_BATCH - 1
}

/// Returns a boolean expressing if the block at a given block number (height) is a micro block.
#[inline]
pub fn is_micro_block_at(block_number: u32) -> bool {
    batch_index_at(block_number) != BLOCKS_PER_BATCH - 1
}

/// Returns the block number of the first block of the given epoch (which is always a micro block).
pub fn first_block_of(epoch: u32) -> u32 {
    if epoch == 0 {
        panic!("Called first_block_of for epoch 0");
    }
    (epoch - 1) * BLOCKS_PER_EPOCH + 1
}

/// Returns the block number of the first block of the given batch (which is always a micro block).
pub fn first_block_of_batch(batch: u32) -> u32 {
    if batch == 0 {
        panic!("Called first_block_of_batch for batch 0");
    }
    (batch - 1) * BLOCKS_PER_BATCH + 1
}

/// Returns the block number of the election macro block of the given epoch (which is always the last block).
pub fn election_block_of(epoch: u32) -> u32 {
    epoch * BLOCKS_PER_EPOCH
}

/// Returns the block number of the macro block (checkpoint or election) of the given batch (which
/// is always the last block).
pub fn macro_block_of(batch: u32) -> u32 {
    batch * BLOCKS_PER_BATCH
}

/// Returns a boolean expressing if the batch at a given block number (height) is the first batch
/// of the epoch.
#[inline]
pub fn first_batch_of_epoch(block_number: u32) -> bool {
    epoch_index_at(block_number) < BLOCKS_PER_BATCH
}

/// Returns the supply at a given time (as Unix time) in Lunas (1 NIM = 100,000 Lunas). It is
/// calculated using the following formula:
/// Supply (t) = Genesis_supply + Initial_supply_velocity / Supply_decay * (1 - e^(- Supply_decay * t))
/// Where e is the exponential function, t is the time in milliseconds since the genesis block and
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
    use super::*;
    use nimiq_test_log::test;

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

    #[test]
    fn it_correctly_computes_first_batch_of_epoch() {
        assert_eq!(first_batch_of_epoch(1), true);
        assert_eq!(first_batch_of_epoch(32), true);
        assert_eq!(first_batch_of_epoch(33), false);
        assert_eq!(first_batch_of_epoch(128), false);
        assert_eq!(first_batch_of_epoch(129), true);
    }
}
