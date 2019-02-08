use std::io;
use std::fmt;

use database::{FromDatabaseValue, IntoDatabaseValue};
use beserial::{Deserialize, Serialize, DeserializeWithLength, SerializeWithLength, SerializingError, ReadBytesExt, WriteBytesExt};


/// Keeps track of the number of superblocks at each level
#[derive(Clone)]
pub struct SuperBlockCounts {
    counts: Vec<u32>
}

impl SuperBlockCounts {
    pub const NUM_COUNTS: usize = 256; // There are 256 levels for Blake2

    /// Create a new SuperBlockCounts with all levels set to 0
    pub fn zero() -> SuperBlockCounts {
        SuperBlockCounts { counts: Vec::new() }
    }

    /// Expands the internal `counts` vector to the desired length `depth`
    fn expand(&mut self, depth: u8) {
        while self.counts.len() <= depth as usize {
            self.counts.push(0);
        }
    }

    /// Increments the superblock count for `depth`
    pub fn add(&mut self, depth: u8) {
        self.expand(depth);
        for i in 0..=(depth as usize) {
            self.counts[i] += 1;
        }
    }

    /// Decrements the superblock count for `depth`
    pub fn substract(&mut self, depth: u8) {
        // NOTE: The `counts` vector must already be longer, otherwise the non-existing entry counts as 0, which we can't substract
        assert!((depth as usize) < self.counts.len());
        for i in 0..=(depth as usize) {
            assert!(self.counts[i] >= 0, format!("Superblock count for level {} is already 0 and can't be decreased", i));
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
    pub fn get(&self, depth: u8) -> u32 {
        if (depth as usize) < self.counts.len() { self.counts[depth as usize] }
        else { 0 } // If the entry is not allocated it the vector it is 0.
    }

    pub fn get_candidate_depth(&self, m: u32) -> u8 {
        // Check that we can actually assume that the result will fit into `u8`
        assert!(Self::NUM_COUNTS - 1 <= (std::u8::MAX as usize));

        if m == 0 {
            return (Self::NUM_COUNTS - 1) as u8;
        }

        /*
        // Get an enumerated iterator over counts and reverse it. This walks backwards over counts
        // and gives us (index, count). Then find the first where count is greater or equal m
        self.counts
            .iter().enumerate().rev()
            .find(|(i, count)| count >= m)
            .unwrap_or(0) as u8
        */

        for (i, count) in self.counts.iter().enumerate().rev() {
            if *count >= m {
                return i as u8;
            }
        }
        // If we haven't returned, there is no candidate -> return 0
        0u8
    }
}

impl Default for SuperBlockCounts {
    fn default() -> SuperBlockCounts {
        SuperBlockCounts::zero()
    }
}

impl PartialEq for SuperBlockCounts {
    fn eq(&self, other: &SuperBlockCounts) -> bool {
        // check if the parts that exists in both, are equal
        self.counts.iter()
            .zip(&other.counts)
            .all(|(&left, right)| left == *right)

        && // then get the remainder which is in one of the both `Vec`s
        if self.counts.len() < other.counts.len() { &other.counts[self.counts.len()..] }
        else { &self.counts[other.counts.len()..] }.iter()
            // and check that those items are 0
            .all(|count| *count == 0)
    }
}

impl Eq for SuperBlockCounts {}

impl fmt::Debug for SuperBlockCounts {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, count) in self.counts.iter().enumerate() {
            write!(f, "{}", count)?;
            if i < Self::NUM_COUNTS - 1 {
                write!(f, ",")?;
            }
        }
        if self.counts.len() < Self::NUM_COUNTS - 1 {
            write!(f, "...");
        }
        write!(f, "]")
    }
}

impl From<Vec<u32>> for SuperBlockCounts {
    fn from(counts: Vec<u32>) -> SuperBlockCounts {
        assert!(counts.len() < Self::NUM_COUNTS, "Vector must not be larger than {} items.", Self::NUM_COUNTS);
        SuperBlockCounts { counts }
    }
}

impl Serialize for SuperBlockCounts {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        SerializeWithLength::serialize::<u8, W>(&self.counts, writer)
    }

    fn serialized_size(&self) -> usize {
        SerializeWithLength::serialized_size::<u8>(&self.counts)
    }
}

impl Deserialize for SuperBlockCounts {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(SuperBlockCounts { counts: DeserializeWithLength::deserialize::<u8, R>(reader)? })
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