use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};
use itertools::{EitherOrBoth, Itertools};
use beserial::{Serialize, Deserialize, uvar, SerializingError, ReadBytesExt, WriteBytesExt, FromPrimitive, ToPrimitive};
use std::fmt;


#[inline]
fn index_and_mask(value: usize) -> (usize, u64) {
    (value >> 3, 1u64 << (value & 63))
}


#[derive(Clone, Eq)]
pub struct BitSet {
    store: Vec<u64>,
    count: usize,
}

impl BitSet {
    pub fn new() -> Self {
        BitSet {
            store: Vec::new(),
            count: 0,
        }
    }

    pub fn with_capacity(nbits: usize) -> Self {
        let nblocks = nbits + if nbits & 63 == 0 { 0 } else { 1 };
        Self {
            store: Vec::with_capacity(nblocks),
            count: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.store.capacity() << 3
    }

    pub fn clear(&mut self) {
        self.store.clear()
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn contains(&self, value: usize) -> bool {
        let (i, m) = index_and_mask(value);
        if let Some(x) = self.store.get(i) {
            x & m != 0
        } else {
            false
        }
    }

    pub fn insert(&mut self, value: usize) {
        let (i, m) = index_and_mask(value);
        if i >= self.store.len() {
            self.store.resize(i + 1, 0);
        }
        if self.store[i] & m == 0 {
            self.count += 1;
        }
        self.store[i] |= m;
    }

    pub fn remove(&mut self, value: usize) {
        let (i, m) = index_and_mask(value);
        if i < self.store.len() {
            if self.store[i] & m != 0 {
                self.count -= 1;
            }
            self.store[i] &= !m;
        }
    }

    fn apply_op<O: Fn(u64, u64) -> u64>(&self, other: Self, op: O) -> Self {
        let mut store: Vec<u64> = Vec::new();
        let mut count: usize = 0;

        for t in self.store.iter().zip_longest(other.store.iter()) {
            let x = match t {
                EitherOrBoth::Both(&a, &b) => op(a, b),
                EitherOrBoth::Left(&a) => op(a, 0),
                EitherOrBoth::Right(&b) => op(0, b),
            };
            store.push(x);
            count += x.count_ones() as usize;
        }

        BitSet { store, count }
    }

    fn apply_op_assign<O: Fn(&mut u64, u64)>(&mut self, other: Self, op: O) {
        let mut count: usize = 0;
        for t in self.store.iter_mut().zip_longest(other.store.iter()) {
            match t {
                EitherOrBoth::Both(a, &b) => {
                    op(a, b);
                    count += a.count_ones() as usize;
                },
                EitherOrBoth::Left(_) => break,
                EitherOrBoth::Right(&b) => {
                    let mut x = 0u64;
                    op(&mut x, b);
                    count += b.count_ones() as usize;
                }
            }
        }
        self.count = count;
    }

    fn iter<'a>(&'a self) -> impl Iterator<Item=usize> + 'a {
        let it = self.store.iter();
        it.enumerate().flat_map(|(i, &block)| {
            let mut values: Vec<usize> = Vec::with_capacity(64);
            let mut j = 0;
            let mut x = block;
            while x > 0 {
                if x & 1 != 0 {
                    values.push(j * 64 + i)
                }
                x >>= 1;
                j += 1
            }
            values.into_iter()
        })
    }
}

/*struct BitSetIter<'a> {
    value: usize,
    block: Option<u64>,
    block_iter: Iter<u64>,
}

impl Iterator for BitSetIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let block = if Some(block) = self.block {
            block
        }
        else {
            if Some(next_block) = self.block_iter.next() {
                self.block = Some(next_block);
                next_block
            }
            else {
                return None
            }
        };


    }
}*/


impl Default for BitSet {
    fn default() -> Self {
        Self::new()
    }
}

impl BitAnd for BitSet {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        self.apply_op(other, BitAnd::bitand)
    }
}

impl BitAndAssign for BitSet {
    fn bitand_assign(&mut self, other: Self) {
        self.apply_op_assign(other, BitAndAssign::bitand_assign)
    }
}

impl BitOr for BitSet {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.apply_op(other, BitOr::bitor)
    }
}

impl BitOrAssign for BitSet {
    fn bitor_assign(&mut self, other: Self) {
        self.apply_op_assign(other, BitOrAssign::bitor_assign)
    }
}

impl PartialEq for BitSet {
    fn eq(&self, other: &Self) -> bool {
        self.store.iter().zip_longest(other.store.iter()).all(|x| match x {
            EitherOrBoth::Both(&a, &b) => a == b,
            EitherOrBoth::Left(&a) => a == 0,
            EitherOrBoth::Right(&b) => b == 0,
        })
    }
}

impl Serialize for BitSet {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += uvar::from_usize(self.store.len())
            .ok_or(SerializingError::Overflow)?
            .serialize(writer)?;
        for x in self.store.iter() {
            size += x.serialize(writer)?
        }
        size += uvar::from_usize(self.count)
            .ok_or(SerializingError::Overflow)?
            .serialize(writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += uvar::from_usize(self.store.len()).unwrap().serialized_size();
        size += self.store.len() * 0u64.serialized_size();
        size += uvar::from_usize(self.count).unwrap().serialized_size();
        size
    }
}

impl Deserialize for BitSet {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let n: uvar = Deserialize::deserialize(reader)?;
        let n = n.to_usize().ok_or(SerializingError::Overflow)?;

        let mut store: Vec<u64> = Vec::new();
        for _ in 0..n {
            store.push(Deserialize::deserialize(reader)?);
        }

        let count: uvar = Deserialize::deserialize(reader)?;
        let count = count.to_usize().ok_or(SerializingError::Overflow)?;

        Ok(BitSet {
            store,
            count,
        })
    }
}

impl fmt::Display for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{{ ")?;
        let mut it = self.iter::<>().peekable();
        while let Some(value) = it.next() {
            write!(f, "{}", value)?;
            if it.peek().is_some() {
                write!(f, ", ")?;
            }
        }
        write!(f, " }}")?;
        Ok(())
    }
}

impl fmt::Debug for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "BitSet")?;
        fmt::Display::fmt(self, f)?;
        Ok(())
    }
}