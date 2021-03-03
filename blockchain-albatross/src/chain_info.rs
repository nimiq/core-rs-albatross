use std::convert::TryInto;
use std::io;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use block::{
    Block, BlockBody, BlockComponents, BlockHeader, BlockJustification, BlockType, MacroBody,
    MacroHeader, MicroBody, MicroHeader, MicroJustification, TendermintProof,
};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;
use primitives::coin::Coin;
use primitives::policy;
use transaction::Transaction;

use crate::SlashPushError;

/// Struct that, for each block, keeps information relative to the chain the block is on.
#[derive(Clone, Debug)]
pub struct ChainInfo {
    // This is the block.
    pub head: Block,
    // A boolean stating if this block is in the main chain.
    pub on_main_chain: bool,
    // The hash of next block in the chain.
    pub main_chain_successor: Option<Blake2bHash>,
    // The sum of all transaction fees in this chain. It resets every batch.
    pub cum_tx_fees: Coin,
}

impl ChainInfo {
    /// Creates the ChainInfo for the genesis block.
    pub fn initial(block: Block) -> Self {
        ChainInfo {
            head: block,
            on_main_chain: true,
            main_chain_successor: None,
            cum_tx_fees: Default::default(),
        }
    }

    /// Creates a new ChainInfo for a block given its predecessor.
    pub fn new(block: Block, prev_info: &ChainInfo) -> Result<Self, SlashPushError> {
        assert_eq!(prev_info.head.block_number(), block.block_number() - 1);

        // Reset the transaction fee accumulator if this is the first block of a batch. Otherwise,
        // just add the transactions fees of this block to the accumulator.
        let cum_tx_fees = if policy::is_macro_block_at(prev_info.head.block_number()) {
            block.sum_transaction_fees()
        } else {
            prev_info.cum_tx_fees + block.sum_transaction_fees()
        };

        Ok(ChainInfo {
            on_main_chain: false,
            main_chain_successor: None,
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
            cum_tx_fees: Default::default(),
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
                    justification: justification
                        .and_then(|justification| Some(BlockJustification::Macro(justification))),
                    body: body.and_then(|body| Some(BlockBody::Macro(body))),
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
                    justification: justification
                        .and_then(|justification| Some(BlockJustification::Micro(justification))),
                    body: body.and_then(|body| Some(BlockBody::Micro(body))),
                }
            }
        }
        // re-create the block out of the BlockComponenets returned by the match
        .try_into()
        .map_err(|_| SerializingError::InvalidValue)?;

        let on_main_chain = Deserialize::deserialize(reader)?;
        let main_chain_successor = Deserialize::deserialize(reader)?;
        let cum_tx_fees = Deserialize::deserialize(reader)?;

        Ok(ChainInfo {
            head,
            on_main_chain,
            main_chain_successor,
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
