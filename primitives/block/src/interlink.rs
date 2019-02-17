use std::io;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Blake2bHash, Hash};
use utils::merkle;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub struct BlockInterlink {
    pub hashes: Vec<Blake2bHash>,
    repeat_bits: Vec<u8>,
    compressed: Vec<Blake2bHash>,
}

impl BlockInterlink {
    #[inline]
    pub fn len(&self) -> usize { self.hashes.len() }

    #[inline]
    pub fn is_empty(&self) -> bool { self.hashes.is_empty() }

    fn compress(hashes: &[Blake2bHash], prev_hash: &Blake2bHash) -> (Vec<u8>, Vec<Blake2bHash>) {
        let repeat_bits_size = if hashes.is_empty() { 0 } else { (hashes.len() - 1) / 8 + 1 };
        let mut repeat_bits = vec![0u8; repeat_bits_size];

        let mut hash = prev_hash;
        let mut compressed: Vec<Blake2bHash> = vec![];

        for i in 0..hashes.len() {
            if &hashes[i] != hash {
                hash = &hashes[i];
                compressed.push(hash.clone());
            } else {
                repeat_bits[(i / 8) as usize] |= 0x80 >> (i % 8);
            }
        }

        (repeat_bits, compressed)
    }

    pub fn new(hashes: Vec<Blake2bHash>, prev_hash: &Blake2bHash) -> Self {
        let (repeat_bits, compressed) = Self::compress(&hashes, prev_hash);
        BlockInterlink { hashes, repeat_bits, compressed }
    }

    pub fn deserialize<R: ReadBytesExt>(reader: &mut R, prev_hash: &Blake2bHash) -> io::Result<Self> {
        let count: u8 = Deserialize::deserialize(reader)?;
        let repeat_bits_size = if count > 0 { (count - 1) / 8 + 1 } else { 0 };
        let mut repeat_bits = vec![0u8; repeat_bits_size as usize];
        reader.read_exact(&mut repeat_bits[..])?;

        let mut hashes = Vec::with_capacity(count as usize);
        let mut compressed: Vec<Blake2bHash> = vec![];

        for i in 0..count {
            let repeated = (repeat_bits[(i / 8) as usize] & (0x80 >> (i % 8))) != 0;
            if !repeated {
                compressed.push(Deserialize::deserialize(reader)?);
            }
            hashes.push(if !compressed.is_empty() { compressed[compressed.len() - 1].clone() } else { prev_hash.clone() });
        }

        Ok(BlockInterlink { hashes, repeat_bits, compressed })
    }
}

impl Serialize for BlockInterlink {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&(self.hashes.len() as u8), writer)?;
        writer.write_all(&self.repeat_bits[..])?;
        size += self.repeat_bits.len();
        for h in &self.compressed {
            size += Serialize::serialize(&h, writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut hash_sizes = 0;
        for h in &self.compressed {
            hash_sizes += Serialize::serialized_size(&h);
        }

        1 + self.repeat_bits.len() + hash_sizes
    }
}

impl BlockInterlink {
    pub fn hash(&self, genesis_hash: Blake2bHash) -> Blake2bHash {
        let mut vec: Vec<Blake2bHash> = Vec::with_capacity(2 + self.compressed.len());
        vec.push(self.repeat_bits.hash());
        vec.push(genesis_hash);
        for h in &self.compressed {
            vec.push(h.clone());
        }
        merkle::compute_root_from_hashes::<Blake2bHash>(&vec)
    }
}
