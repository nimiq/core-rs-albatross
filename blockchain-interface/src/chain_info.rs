use std::{fmt::Formatter, ops::RangeFrom};

use nimiq_block::{
    Block, BlockError, BlockType, MacroBlock, MacroHeader, MicroBlock, MicroHeader,
    MicroJustification, TendermintProof,
};
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{coin::Coin, key_nibbles::KeyNibbles, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use serde::{
    de::{Error, SeqAccess, Visitor},
    ser::SerializeStruct,
    Deserializer, Serializer,
};

/// Struct that, for each block, keeps information relative to the chain the block is on.
#[derive(Clone, Debug, Serialize, Deserialize, DbSerializable)]
pub struct ChainInfo {
    /// This is the block (excluding the body).
    #[serde(
        serialize_with = "ChainInfo::serialize_head",
        deserialize_with = "ChainInfo::deserialize_head"
    )]
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
    ///
    /// Returns an error if the transaction fee sum overflows.
    pub fn from_block(
        block: Block,
        prev_info: &ChainInfo,
        prev_missing_range: Option<RangeFrom<KeyNibbles>>,
    ) -> Result<Self, BlockError> {
        assert_eq!(prev_info.head.block_number(), block.block_number() - 1);

        // Reset the transaction fee accumulator if this is the first block of a batch. Otherwise,
        // just add the transactions fees of this block to the accumulator.
        let block_fees = block.sum_transaction_fees()?;
        let cum_tx_fees = if Policy::is_macro_block_at(prev_info.head.block_number()) {
            block_fees
        } else {
            prev_info
                .cum_tx_fees
                .checked_add(block_fees)
                .ok_or(BlockError::TransactionFeeOverflow)?
        };

        let prunable = !block.is_election();

        Ok(ChainInfo {
            on_main_chain: false,
            main_chain_successor: None,
            head: block,
            cum_tx_fees,
            cum_hist_tx_size: 0,
            history_tree_len: 0,
            prunable,
            prev_missing_range,
        })
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

    fn serialize_head<S: Serializer>(block: &Block, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("ChainHead", CHAIN_HEAD_FIELDS.len())?;
        state.serialize_field(CHAIN_HEAD_FIELDS[0], &block.ty())?;
        match block {
            Block::Macro(block) => {
                state.serialize_field(CHAIN_HEAD_FIELDS[1], &block.header)?;
                state.serialize_field(CHAIN_HEAD_FIELDS[2], &block.justification)?;
            }
            Block::Micro(block) => {
                state.serialize_field(CHAIN_HEAD_FIELDS[1], &block.header)?;
                state.serialize_field(CHAIN_HEAD_FIELDS[2], &block.justification)?;
            }
        }
        state.end()
    }

    fn deserialize_head<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Block, D::Error> {
        deserializer.deserialize_struct("ChainHead", CHAIN_HEAD_FIELDS, ChainHeadVisitor)
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

const CHAIN_HEAD_FIELDS: &[&str] = &["type", "header", "justification"];

struct ChainHeadVisitor;

impl<'de> Visitor<'de> for ChainHeadVisitor {
    type Value = Block;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("struct Block")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let ty: BlockType = seq
            .next_element()?
            .ok_or_else(|| Error::invalid_length(0, &self))?;

        let block = match ty {
            BlockType::Macro => {
                let header: MacroHeader = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(1, &self))?;
                let justification: Option<TendermintProof> = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(2, &self))?;
                Block::Macro(MacroBlock {
                    header,
                    justification,
                    body: None,
                })
            }
            BlockType::Micro => {
                let header: MicroHeader = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(1, &self))?;
                let justification: Option<MicroJustification> = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(2, &self))?;
                Block::Micro(MicroBlock {
                    header,
                    justification,
                    body: None,
                })
            }
        };

        Ok(block)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_block::{BlockError, MicroBlock, MicroBody, MicroHeader};
    use nimiq_keys::Address;
    use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
    use nimiq_transaction::ExecutedTransaction;

    use super::*;

    fn tx_with_fee(fee: Coin) -> ExecutedTransaction {
        ExecutedTransaction::Ok(nimiq_transaction::Transaction::new_basic(
            Address::default(),
            Address::from([1u8; 20]),
            Coin::ZERO,
            fee,
            1,
            NetworkId::UnitAlbatross,
        ))
    }

    #[test]
    fn test_chain_info_cumulative_fee_overflow() {
        // Use block_number 2 so it's NOT a macro block predecessor, triggering the
        // cumulative addition path.
        let prev_block = Block::Micro(MicroBlock {
            header: MicroHeader {
                network: NetworkId::UnitAlbatross,
                version: Policy::max_supported_version(),
                block_number: 2,
                timestamp: 0,
                ..Default::default()
            },
            justification: None,
            body: None,
        });

        let mut prev_info = ChainInfo::new(prev_block, true);
        prev_info.cum_tx_fees = Coin::MAX;

        // Create a block with fee = 1. Adding to MAX should overflow.
        let transactions = vec![tx_with_fee(Coin::from_u64_unchecked(1))];
        let micro_body = MicroBody {
            equivocation_proofs: vec![],
            transactions,
        };
        let block = Block::Micro(MicroBlock {
            header: MicroHeader {
                network: NetworkId::UnitAlbatross,
                version: Policy::max_supported_version(),
                block_number: 3,
                timestamp: 1,
                ..Default::default()
            },
            justification: None,
            body: Some(micro_body),
        });

        assert!(
            matches!(
                ChainInfo::from_block(block, &prev_info, None),
                Err(BlockError::TransactionFeeOverflow)
            ),
            "Expected TransactionFeeOverflow for cumulative fee overflow",
        );
    }
}
