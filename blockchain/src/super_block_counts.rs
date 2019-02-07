use std::io;
use std::fmt;

use database::{FromDatabaseValue, IntoDatabaseValue};
use beserial::{Deserialize, Serialize, SerializingError, ReadBytesExt, WriteBytesExt};


/// Keeps track of the number of superblocks at each level
#[derive(Clone)]
pub struct SuperBlockCounts {
    counts: [u64; Self::NUM_COUNTS]
}

impl SuperBlockCounts {
    const NUM_COUNTS: usize = 256; // There are 256 levels for Blake2

    /// Create a new SuperBlockCounts with all levels set to 0
    pub fn zero() -> SuperBlockCounts {
        SuperBlockCounts::from([0; Self::NUM_COUNTS])
    }

    /// Increments the superblock count for `depth`
    pub fn add(&mut self, depth: u8) {
        for i in 0..(depth as usize){
            self.counts[i] += 1;
        }
    }

    /// Decrements the superblock count for `depth`
    pub fn substract(&mut self, depth: u8) {
        for i in 0..(depth as usize) {
            assert!(self.counts[i] >= 0, "Superblock count is already 0 and can't be decreased");
            self.counts[i] -= 1;
        }
    }

    /// Increments the superblock count for `depth` and returns the result in a new `SuperBlockCounts`
    pub fn copy_and_add(&self, depth: u8) -> SuperBlockCounts {
        let mut c = self.clone();
        c.add(depth);
        c
    }

    /// Decrements the superblock count for `depth` and returns the result in a new `SuperBlockCounts`
    pub fn copy_and_substract(&self, depth: u8) -> SuperBlockCounts {
        let mut c = self.clone();
        c.substract(depth);
        c
    }

    /// Returns the super block count at `depth`
    pub fn get(&self, depth: u8) -> u64 {
        self.counts[depth as usize]
    }

    pub fn get_candidate_depth(&self, m: u64) -> u8 {
        // NOTE: The cast to u8 is safe, since our indices only go until 255
        assert!(Self::NUM_COUNTS < (std::u8::MAX as usize));

        (0..Self::NUM_COUNTS).map(|i| Self::NUM_COUNTS - i - 1)
            .find(|i| self.counts[i.clone()] >= m)
            .unwrap_or(0) as u8
    }
}

impl Default for SuperBlockCounts {
    fn default() -> SuperBlockCounts {
        SuperBlockCounts::zero()
    }
}

impl PartialEq for SuperBlockCounts {
    fn eq(&self, other: &SuperBlockCounts) -> bool {
        (0..Self::NUM_COUNTS).all(|i| self.counts[i] == other.counts[i])
    }
}

impl Eq for SuperBlockCounts {}

impl fmt::Debug for SuperBlockCounts {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}]", self.counts[0])?;
        for i in 1..Self::NUM_COUNTS {
            write!(f, ",{}", self.counts[i])?;
        }
        write!(f, "]")
    }
}

impl From<[u64; Self::NUM_COUNTS]> for SuperBlockCounts {
    fn from(counts: [u64; SuperBlockCounts::NUM_COUNTS]) -> SuperBlockCounts {
        SuperBlockCounts { counts }
    }
}

impl Serialize for SuperBlockCounts {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        for i in 0..Self::NUM_COUNTS {
            size += Serialize::serialize(&self.counts[i], writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        for i in 0..Self::NUM_COUNTS {
            size += Serialize::serialized_size(&self.counts[i]);
        }
        size
    }
}

impl Deserialize for SuperBlockCounts {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let mut counts: [u64; Self::NUM_COUNTS] = [0; Self::NUM_COUNTS];
        for i in 0..Self::NUM_COUNTS {
            counts[i] = Deserialize::deserialize(reader)?;
        }
        Ok(SuperBlockCounts::from(counts))
    }
}


impl IntoDatabaseValue for SuperBlockCounts {
    fn database_byte_size(&self) -> usize {
        return self.serialized_size();
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for SuperBlockCounts {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        return Ok(Deserialize::deserialize(&mut cursor)?);
    }
}