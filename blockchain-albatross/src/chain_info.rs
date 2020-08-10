use std::io;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use block::{Block, BlockType, MacroBody, MicroBody};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;
use primitives::coin::Coin;
use primitives::policy;
use transaction::Transaction;

use crate::slots::{apply_slashes, ForkProofInfos, SlashPushError, SlashedSet};

#[derive(Clone, Debug)]
pub struct ChainInfo {
    pub head: Block,
    pub on_main_chain: bool,
    pub main_chain_successor: Option<Blake2bHash>,
    pub slashed_set: SlashedSet,
    // Resets every batch
    pub cum_tx_fees: Coin,
}

impl ChainInfo {
    /// Creates the ChainInfo for the genesis block.
    pub fn initial(block: Block) -> Self {
        ChainInfo {
            head: block,
            on_main_chain: true,
            main_chain_successor: None,
            slashed_set: Default::default(),
            cum_tx_fees: Default::default(),
        }
    }

    /// Creates a new ChainInfo for a block given its predecessor.
    /// We need the ForkProofInfos to retrieve the slashed sets just before a fork proof.
    pub fn new(
        block: Block,
        prev_info: &ChainInfo,
        fork_proof_infos: &ForkProofInfos,
    ) -> Result<Self, SlashPushError> {
        assert_eq!(prev_info.head.block_number(), block.block_number() - 1);
        let cum_tx_fees = if policy::is_macro_block_at(prev_info.head.block_number()) {
            block.sum_transaction_fees()
        } else {
            prev_info.cum_tx_fees + block.sum_transaction_fees()
        };

        Ok(ChainInfo {
            on_main_chain: false,
            main_chain_successor: None,
            slashed_set: apply_slashes(&block, prev_info, fork_proof_infos)?,
            head: block,
            cum_tx_fees,
        })
    }

    /// Creates a new dummy ChainInfo for a block ignoring slashes and transaction fees.
    pub fn dummy(block: Block) -> Self {
        ChainInfo {
            head: block,
            on_main_chain: false,
            main_chain_successor: None,
            slashed_set: Default::default(),
            cum_tx_fees: Default::default(),
        }
    }

    /// Calculates the base for the next block's slashed set.
    /// If the current block is an election block, the next slashed set needs to account for
    /// the new epoch. Otherwise, it is simply the current slashed set.
    ///
    /// Note that this method does not yet add new slashes to the set!
    pub fn next_slashed_set(&self) -> SlashedSet {
        self.slashed_set
            .next_slashed_set(self.head.block_number() + 1)
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
// XXX Move this into Block.serialize_xxx()?
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
        size += Serialize::serialize(&self.slashed_set, writer)?;
        size += Serialize::serialize(&self.cum_tx_fees, writer)?;
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
        size += Serialize::serialized_size(&self.slashed_set);
        size += Serialize::serialized_size(&self.cum_tx_fees);
        size
    }
}

impl Deserialize for ChainInfo {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: BlockType = Deserialize::deserialize(reader)?;
        let head = match ty {
            BlockType::Macro => Block::Macro(Deserialize::deserialize(reader)?),
            BlockType::Micro => Block::Micro(Deserialize::deserialize(reader)?),
        };
        let on_main_chain = Deserialize::deserialize(reader)?;
        let main_chain_successor = Deserialize::deserialize(reader)?;
        let slashed_set = Deserialize::deserialize(reader)?;
        let cum_tx_fees = Deserialize::deserialize(reader)?;

        Ok(ChainInfo {
            head,
            on_main_chain,
            main_chain_successor,
            slashed_set,
            cum_tx_fees,
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

pub enum EpochTransactions<'a> {
    Borrowed(&'a [Transaction]),
    Owned(Vec<Transaction>),
}

impl<'a> EpochTransactions<'a> {
    pub fn len(&self) -> u32 {
        match self {
            EpochTransactions::Borrowed(txs) => txs.len() as u32,
            EpochTransactions::Owned(txs) => txs.len() as u32,
        }
    }
}

impl<'a> Serialize for EpochTransactions<'a> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.len(), writer)?;
        match self {
            EpochTransactions::Borrowed(txs) => {
                for tx in txs.iter() {
                    size += Serialize::serialize(tx, writer)?;
                }
            }
            EpochTransactions::Owned(txs) => {
                for tx in txs.iter() {
                    size += Serialize::serialize(tx, writer)?;
                }
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.len());
        match self {
            EpochTransactions::Borrowed(txs) => {
                for tx in txs.iter() {
                    size += Serialize::serialized_size(tx);
                }
            }
            EpochTransactions::Owned(txs) => {
                for tx in txs.iter() {
                    size += Serialize::serialized_size(tx);
                }
            }
        }
        size
    }
}

impl<'a> Deserialize for EpochTransactions<'a> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let len: u32 = Deserialize::deserialize(reader)?;
        let mut txs = vec![];

        for _ in 0..len {
            txs.push(Deserialize::deserialize(reader)?);
        }

        Ok(EpochTransactions::Owned(txs))
    }
}

impl<'a> IntoDatabaseValue for EpochTransactions<'a> {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl<'a> FromDatabaseValue for EpochTransactions<'a> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
