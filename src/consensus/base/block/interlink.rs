use beserial::{Deserialize, ReadBytesExt, Serialize, WriteBytesExt};
use consensus::base::primitive::hash::Blake2bHash;
use std::io;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub struct BlockInterlink(Vec<Blake2bHash>);

impl BlockInterlink {
    pub fn len(&self) -> usize {
        return self.0.len();
    }

    pub fn deserialize<R: ReadBytesExt>(reader: &mut R, prev_hash: &Blake2bHash) -> io::Result<Self> {
        let count: u8 = Deserialize::deserialize(reader)?;
        let repeat_bits_size = if count > 0 { (count - 1) / 8 + 1 } else { 0 };
        let mut repeat_bits = vec![0u8; repeat_bits_size as usize];
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
            if &self.0[i] != hash {
                hash = &self.0[i];
                compressed.push(self.0[i].clone());
            } else {
                repeat_bits[(i / 8) as usize] |= 0x80 >> (i % 8);
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
