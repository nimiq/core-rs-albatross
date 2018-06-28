use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, WriteBytesExt};
use beserial_derive;
use consensus::base::account::PrunedAccount;
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::Blake2bHash;
use consensus::base::transaction::Transaction;
use std::io;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct TargetCompact(u32);

impl From<TargetCompact> for u32 {
    fn from(t: TargetCompact) -> Self {
        return t.0;
    }
}

impl From<u32> for TargetCompact {
    fn from(u: u32) -> Self {
        return TargetCompact(u);
    }
}

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
pub struct BlockInterlink(Vec<Blake2bHash>);

impl BlockInterlink {
    pub fn len(&self) -> usize {
        return self.0.len();
    }

    pub fn deserialize<R: ReadBytesExt>(reader: &mut R, prev_hash: &Blake2bHash) -> io::Result<Self> {
        let count: u8 = Deserialize::deserialize(reader)?;
        let repeat_bits_size = if count > 0 { (count - 1) / 8 + 1 } else { 0 };
        let mut repeat_bits = Vec::with_capacity(repeat_bits_size as usize);
        reader.read_exact(&mut repeat_bits[..])?;

        let mut hash: Option<Blake2bHash> = Option::None;
        let mut hashes = Vec::with_capacity(count as usize);

        for i in 0..count {
            let repeated = (repeat_bits[(i / 8) as usize] & (0x80 >> (i % 8))) != 0;
            if !repeated {
                hash = Option::Some(Deserialize::deserialize(reader)?);
            }
            hashes.push(hash.clone().unwrap_or_else(|| prev_hash.clone()).clone());
        }

        return Ok(BlockInterlink(hashes));
    }

    pub fn serialize<W: WriteBytesExt>(&self, writer: &mut W, prev_hash: &Blake2bHash) -> io::Result<usize> {
        let repeat_bits_size = if self.0.len() > 0 { (self.0.len() - 1) / 8 + 1 } else { 0 };
        let mut repeat_bits = vec![0u8; repeat_bits_size];

        let mut hash = prev_hash;
        let mut compressed = vec![];

        for i in 0..self.0.len() {
            if &self.0[i] != prev_hash {
                hash = &self.0[i];
                compressed.push(self.0[i].clone());
            } else {
                repeat_bits[(i / 8) as usize] |= (0x80 >> (i % 8));
            }
        };

        let mut size = 0;
        size += Serialize::serialize(&(self.0.len() as u8), writer)?;
        writer.write_all(&repeat_bits[..])?;
        size += repeat_bits_size;
        for h in compressed {
            size += Serialize::serialize(&h, writer)?;
        }
        return Ok(size);
    }

    pub fn serialized_size(&self, prev_hash: &Blake2bHash) -> usize {
        let repeat_bits_size = if self.0.len() > 0 { (self.0.len() - 1) / 8 + 1 } else { 0 };

        let mut hash: &Blake2bHash = prev_hash;
        let mut hash_sizes = 0;
        for i in 0..self.0.len() {
            if &self.0[i] != hash {
                hash = &self.0[i];
                hash_sizes += hash.serialized_size();
            }
        }

        return 1 + repeat_bits_size + hash_sizes;
    }
}

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
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

impl Serialize for Block {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        size += Serialize::serialize(&self.header, writer)?;
        size += self.interlink.serialize(writer, &self.header.prev_hash)?;
        size += Serialize::serialize(&self.body, writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.header);
        size += self.interlink.serialized_size(&self.header.prev_hash);
        size += Serialize::serialized_size(&self.body);
        return size;
    }
}
