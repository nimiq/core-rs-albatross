use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, WriteBytesExt};
use beserial_derive;
use consensus::base::account::PrunedAccount;
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::Blake2bHash;
use consensus::base::transaction::Transaction;
use std::io;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct TargetCompact(u32);

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u16,
    pub prev_hash: Blake2bHash,
    pub interlink_hash: Blake2bHash,
    pub body_hash: Blake2bHash,
    pub accounts_hash: Blake2bHash,
    pub n_bits: TargetCompact,
    pub height: u32,
    pub timestamp: u32,
    pub nonce: u32,
}

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BlockBody {
    pub miner: Address,
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    #[beserial(len_type(u16))]
    pub pruned_accounts: Vec<PrunedAccount>,
}

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub struct BlockInterlink {
    pub hashes: Vec<Blake2bHash>,
    repeat_bits: Vec<u8>,
    compressed: Vec<Blake2bHash>,
}

impl BlockInterlink {
    pub fn deserialize<R: ReadBytesExt>(reader: &mut R, prev_hash: &Blake2bHash) -> io::Result<Self> {
        let count: u8 = Deserialize::deserialize(reader)?;
        let repeat_bits_size = (count - 1) / 8 + 1;
        let mut repeat_bits = Vec::with_capacity(repeat_bits_size as usize);
        reader.read_exact(&mut repeat_bits[..])?;

        let mut hash: Option<Blake2bHash> = Option::None;
        let mut hashes = Vec::with_capacity(count as usize);
        let mut compressed = vec![];

        for i in 0..count {
            let repeated = (repeat_bits[(i / 8) as usize] & (0x80 >> (i % 8))) != 0;
            if !repeated {
                hash = Option::Some(Deserialize::deserialize(reader)?);
                compressed.push(hash.clone().unwrap());
            }
            hashes.push(hash.clone().unwrap_or_else(|| prev_hash.clone()).clone());
        }

        return Ok(BlockInterlink { hashes, repeat_bits, compressed });
    }
}

impl Serialize for BlockInterlink {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize)]
pub struct Block {
    pub header: BlockHeader,
    pub interlink: BlockInterlink,
    pub body: Option<BlockBody>,
}

impl Deserialize for Block {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let header: BlockHeader = Deserialize::deserialize(reader)?;
        let interlink = BlockInterlink::deserialize(reader, &header.prev_hash)?;
        return Ok(Block {
            header,
            interlink,
            body: Deserialize::deserialize(reader)?,
        });
    }
}
