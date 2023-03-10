use std::convert::TryInto;
use std::io;
use std::ops::RangeFrom;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_block::{
    Block, BlockBody, BlockComponents, BlockHeader, BlockJustification, BlockType, MacroBody,
    MacroHeader, MicroBody, MicroHeader, MicroJustification, TendermintProof,
};
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{coin::Coin, key_nibbles::KeyNibbles, policy::Policy};

/// Struct that, for each block, keeps information relative to the chain the block is on.
#[derive(Clone, Debug)]
pub struct ChainInfo {
    /// This is the block (excluding the body).
    pub head: Block,
    /// A boolean stating if this block is in the main chain.
    pub on_main_chain: bool,
    /// The hash of next block in the chain.
    pub main_chain_successor: Option<Blake2bHash>,
    /// The sum of all transaction fees in this chain. It resets every batch.
    pub cum_tx_fees: Coin,
    /// The accumulated extended transaction size. It resets every other macro block.
    pub cum_ext_tx_size: u64,
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
            cum_ext_tx_size: 0,
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
            cum_ext_tx_size: 0,
            history_tree_len: 0,
            prunable,
            prev_missing_range,
        }
    }

    /// Sets the value for the cumulative extended transaction size and the prunable flag
    pub fn set_cumulative_ext_tx_size(&mut self, prev_info: &ChainInfo, block_ext_tx_size: u64) {
        // Reset the cumulative extended transaction size if the previous block
        // was an election macro block or it was a checkpoint macro block and such checkpoint macro
        // block was not pruned.
        // Also set prunable to `false` if we exceeded the policy::HISTORY_CHUNKS_MAX_SIZE
        if Policy::is_election_block_at(prev_info.head.block_number())
            || (Policy::is_macro_block_at(prev_info.head.block_number()) && !prev_info.prunable)
        {
            self.cum_ext_tx_size = block_ext_tx_size;
            self.prunable = true;
        } else if Policy::is_macro_block_at(self.head.block_number())
            && prev_info.cum_ext_tx_size + block_ext_tx_size > Policy::HISTORY_CHUNKS_MAX_SIZE
        {
            self.cum_ext_tx_size = prev_info.cum_ext_tx_size + block_ext_tx_size;
            self.prunable = false;
        } else {
            self.cum_ext_tx_size = prev_info.cum_ext_tx_size + block_ext_tx_size;
            self.prunable = true;
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

// Do not serialize the block body.
// TODO: Move this into Block.serialize_xxx()?
impl Serialize for ChainInfo {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.head.ty(), writer)?;
        match self.head {
            Block::Macro(ref macro_block) => {
                size += Serialize::serialize(&macro_block.header, writer)?;
                size += Serialize::serialize(&macro_block.justification, writer)?;
                size += Serialize::serialize(&None::<MacroBody>, writer)?;
            }
            Block::Micro(ref micro_block) => {
                size += Serialize::serialize(&micro_block.header, writer)?;
                size += Serialize::serialize(&micro_block.justification, writer)?;
                size += Serialize::serialize(&None::<MicroBody>, writer)?;
            }
        }
        size += Serialize::serialize(&self.on_main_chain, writer)?;
        size += Serialize::serialize(&self.main_chain_successor, writer)?;
        size += Serialize::serialize(&self.cum_tx_fees, writer)?;
        size += Serialize::serialize(&self.cum_ext_tx_size, writer)?;
        size += Serialize::serialize(&self.history_tree_len, writer)?;
        size += Serialize::serialize(&self.prunable, writer)?;
        size += Serialize::serialize(&self.prev_missing_range, writer)?;

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.head.ty());
        match self.head {
            Block::Macro(ref macro_block) => {
                size += Serialize::serialized_size(&macro_block.header);
                size += Serialize::serialized_size(&macro_block.justification);
                size += Serialize::serialized_size(&None::<MacroBody>);
            }
            Block::Micro(ref micro_block) => {
                size += Serialize::serialized_size(&micro_block.header);
                size += Serialize::serialized_size(&micro_block.justification);
                size += Serialize::serialized_size(&None::<MicroBody>);
            }
        }
        size += Serialize::serialized_size(&self.on_main_chain);
        size += Serialize::serialized_size(&self.main_chain_successor);
        size += Serialize::serialized_size(&self.cum_tx_fees);
        size += Serialize::serialized_size(&self.cum_ext_tx_size);
        size += Serialize::serialized_size(&self.history_tree_len);
        size += Serialize::serialized_size(&self.prunable);
        size += Serialize::serialized_size(&self.prev_missing_range);
        size
    }
}

impl Deserialize for ChainInfo {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let head = match ty {
            BlockType::Macro => {
                // Maintain correct order of serialized objects here such that the deserialization
                // does not rely on the struct being in the correct order.
                let header: MacroHeader = Deserialize::deserialize(reader)?;
                let justification: Option<TendermintProof> = Deserialize::deserialize(reader)?;
                let body: Option<MacroBody> = Deserialize::deserialize(reader)?;

                // Group the deserialized parts in a BlockComponents instance.
                BlockComponents {
                    header: Some(BlockHeader::Macro(header)),
                    justification: justification.map(BlockJustification::Macro),
                    body: body.map(BlockBody::Macro),
                }
            }
            BlockType::Micro => {
                // Maintain correct order of serialized objects here such that the deserialization
                // does not rely on the struct being in the correct order.
                let header: MicroHeader = Deserialize::deserialize(reader)?;
                let justification: Option<MicroJustification> = Deserialize::deserialize(reader)?;
                let body: Option<MicroBody> = Deserialize::deserialize(reader)?;

                // Group the deserialized parts in a BlockComponents instance.
                BlockComponents {
                    header: Some(BlockHeader::Micro(header)),
                    justification: justification.map(BlockJustification::Micro),
                    body: body.map(BlockBody::Micro),
                }
            }
        }
        // re-create the block out of the BlockComponents returned by the match
        .try_into()
        .map_err(|_| SerializingError::InvalidValue)?;

        let on_main_chain = Deserialize::deserialize(reader)?;
        let main_chain_successor = Deserialize::deserialize(reader)?;
        let cum_tx_fees = Deserialize::deserialize(reader)?;
        let cum_ext_tx_size = Deserialize::deserialize(reader)?;
        let history_tree_len = Deserialize::deserialize(reader)?;
        let prunable = Deserialize::deserialize(reader)?;
        let prev_missing_range = Deserialize::deserialize(reader)?;

        Ok(ChainInfo {
            head,
            on_main_chain,
            main_chain_successor,
            cum_tx_fees,
            cum_ext_tx_size,
            history_tree_len,
            prunable,
            prev_missing_range,
        })
    }
}

impl IntoDatabaseValue for ChainInfo {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ChainInfo {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
