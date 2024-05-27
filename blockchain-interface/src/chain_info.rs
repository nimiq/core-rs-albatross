use std::{io, ops::RangeFrom};

use nimiq_block::Block;
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{coin::Coin, key_nibbles::KeyNibbles, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};

/// Struct that, for each block, keeps information relative to the chain the block is on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainInfo {
    /// This is the block (excluding the body).
    #[serde(serialize_with = "ChainInfo::serialize_head")]
    pub head: Block,
    /// A boolean stating if this block is in the main chain.
    pub on_main_chain: bool,
    /// The hash of next block in the chain.
    pub main_chain_successor: Option<Blake2bHash>,
    /// The sum of all transaction fees in this chain. It resets every batch.
    pub cum_tx_fees: Coin,
    /// The accumulated historic transaction size. It resets every other macro block.
    pub cum_hist_tx_size: u64,
    /// The total length of the history tree up to the current block.
    pub history_tree_len: u64,
    /// A boolean stating if this block can be pruned.
    pub prunable: bool,
    /// Missing range of the accounts before this block.
    pub prev_missing_range: Option<RangeFrom<KeyNibbles>>,
}

impl ChainInfo {
    /// Creates a new ChainInfo. Without successor and cumulative transaction fees.
    /// Note: This sets the previous missing range to none. This is ok since this function
    /// is only used for the genesis block.
    pub fn new(head: Block, on_main_chain: bool) -> Self {
        let prunable = !head.is_election();
        ChainInfo {
            head,
            on_main_chain,
            main_chain_successor: None,
            cum_tx_fees: Coin::ZERO,
            cum_hist_tx_size: 0,
            history_tree_len: 0,
            prunable,
            prev_missing_range: None,
        }
    }

    /// Creates a new ChainInfo for a block given its predecessor.
    pub fn from_block(
        block: Block,
        prev_info: &ChainInfo,
        prev_missing_range: Option<RangeFrom<KeyNibbles>>,
    ) -> Self {
        assert_eq!(prev_info.head.block_number(), block.block_number() - 1);

        // Reset the transaction fee accumulator if this is the first block of a batch. Otherwise,
        // just add the transactions fees of this block to the accumulator.
        let cum_tx_fees = if Policy::is_macro_block_at(prev_info.head.block_number()) {
            block.sum_transaction_fees()
        } else {
            prev_info.cum_tx_fees + block.sum_transaction_fees()
        };

        let prunable = !block.is_election();

        ChainInfo {
            on_main_chain: false,
            main_chain_successor: None,
            head: block,
            cum_tx_fees,
            cum_hist_tx_size: 0,
            history_tree_len: 0,
            prunable,
            prev_missing_range,
        }
    }

    /// Sets the value for the cumulative historic transaction size and the prunable flag
    pub fn set_cumulative_hist_tx_size(&mut self, prev_info: &ChainInfo, block_hist_tx_size: u64) {
        // Reset the cumulative historic transaction size if the previous block
        // was an election macro block or it was a checkpoint macro block and such checkpoint macro
        // block was not pruned.
        // Also set prunable to `false` if we exceeded the policy::HISTORY_CHUNKS_MAX_SIZE
        if Policy::is_election_block_at(prev_info.head.block_number())
            || (Policy::is_macro_block_at(prev_info.head.block_number()) && !prev_info.prunable)
        {
            self.cum_hist_tx_size = block_hist_tx_size;
            self.prunable = true;
        } else if Policy::is_macro_block_at(self.head.block_number())
            && prev_info.cum_hist_tx_size + block_hist_tx_size > Policy::HISTORY_CHUNKS_MAX_SIZE
        {
            self.cum_hist_tx_size = prev_info.cum_hist_tx_size + block_hist_tx_size;
            self.prunable = false;
        } else {
            self.cum_hist_tx_size = prev_info.cum_hist_tx_size + block_hist_tx_size;
            self.prunable = true;
        }
    }

    fn serialize_head<S>(block: &Block, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Empty body if there is some
        if block.body().is_some() {
            let mut block_to_ser = block.clone();
            match block_to_ser {
                Block::Macro(ref mut block) => block.body = None,
                Block::Micro(ref mut block) => block.body = None,
            }
            serde::Serialize::serialize(&block_to_ser, serializer)
        } else {
            serde::Serialize::serialize(&block, serializer)
        }
    }
}

impl PartialEq for ChainInfo {
    fn eq(&self, other: &Self) -> bool {
        self.head.eq(&other.head)
            && self.on_main_chain == other.on_main_chain
            && self.main_chain_successor.eq(&other.main_chain_successor)
    }
}

impl Eq for ChainInfo {}

impl IntoDatabaseValue for ChainInfo {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize_to_writer(self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ChainInfo {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Deserialize::deserialize_from_vec(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}
