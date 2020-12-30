use std::fmt;
use std::iter::{repeat, FromIterator};
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};

use itertools::{EitherOrBoth, Itertools};

use beserial::{uvar, Deserialize, FromPrimitive, ReadBytesExt, Serialize, SerializingError, ToPrimitive, WriteBytesExt};

#[inline]
fn index_and_mask(value: usize) -> (usize, u64) {
    (value >> 6, 1u64 << (value & 63))
}

#[derive(Clone, Eq)]
pub struct BitSet {
    store: Vec<u64>,
    count: usize,
}

impl BitSet {
    pub fn new() -> Self {
        BitSet { store: Vec::new(), count: 0 }
    }

    pub fn with_capacity(nbits: usize) -> Self {
        let nblocks = (nbits >> 6) + if nbits.trailing_zeros() >= 6 { 0 } else { 1 };
        Self {
            store: Vec::with_capacity(nblocks),
            count: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.store.capacity() << 3
    }

    pub fn clear(&mut self) {
        self.store.clear();
        self.count = 0;
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
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
        self.compact();
    }

    fn apply_op<O: Fn(u64, u64) -> u64>(&self, other: &Self, op: O) -> Self {
        let mut store: Vec<u64> = Vec::new();
        let mut count: usize = 0;

        for t in self.store.iter().zip_longest(other.store.iter()) {
            let x = match t {
                EitherOrBoth::Both(a, b) => op(*a, *b),
                EitherOrBoth::Left(a) => op(*a, 0),
                EitherOrBoth::Right(b) => op(0, *b),
            };
            store.push(x);
            count += x.count_ones() as usize;
        }

        let mut bitset = BitSet { store, count };
        bitset.compact();
        bitset
    }

    fn apply_op_assign<O: Fn(&mut u64, u64)>(&mut self, other: Self, op: O) {
        let mut count: usize = 0;
        let mut new = vec![];

        for t in self.store.iter_mut().zip_longest(other.store.iter()) {
            match t {
                EitherOrBoth::Both(a, b) => {
                    op(a, *b);
                    count += a.count_ones() as usize;
                }
                EitherOrBoth::Left(a) => {
                    let x = 0u64;
                    op(a, x);
                    count += a.count_ones() as usize;
                }
                EitherOrBoth::Right(b) => {
                    let mut x = 0u64;
                    op(&mut x, *b);
                    new.push(x);
                    count += x.count_ones() as usize;
                }
            }
        }

        self.store.append(&mut new);

        self.count = count;
        self.compact();
    }

    /// Removes unnecessary parts from store.
    #[inline]
    fn compact(&mut self) {
        let num_empty = self.store.iter().rev().take_while(|&v| *v == 0).count();
        self.store.truncate(self.store.len() - num_empty);
    }

    /// Infinite iterator of excluded items
    pub fn iter_excluded<'a>(&'a self) -> impl Iterator<Item = usize> + 'a {
        self.iter_bits().enumerate().filter_map(|(i, one)| if one { None } else { Some(i) })
    }

    /// Iterator of included items
    pub fn iter<'a>(&'a self) -> impl Iterator<Item = usize> + 'a {
        self.iter_bits()
            .take(self.store.len() * 64)
            .enumerate()
            .filter_map(|(i, one)| if one { Some(i) } else { None })
    }

    /// Infinite iterator of bits
    pub fn iter_bits<'a>(&'a self) -> impl Iterator<Item = bool> + 'a {
        self.store.iter().flat_map(|store| Bits64Iter::new(*store)).chain(repeat(false))
    }

    /// Test if this is a superset of `other`
    pub fn is_superset(&self, other: &Self) -> bool {
        for t in self.store.iter().zip_longest(other.store.iter()) {
            let c = match t {
                EitherOrBoth::Both(&a, &b) => a & b == b,
                EitherOrBoth::Left(_) => {
                    // Since the left side can't change the result anymore, it must be a superset
                    return true;
                }
                EitherOrBoth::Right(&b) => b == 0,
            };
            if !c {
                return false;
            }
        }
        true
    }

    /// Test if this is a subset of `other`
    pub fn is_subset(&self, other: &Self) -> bool {
        other.is_superset(self)
    }

    pub fn intersection_size(&self, other: &Self) -> usize {
        self.store.iter().zip(other.store.iter()).map(|(a, b)| (a & b).count_ones() as usize).sum()
    }
}

impl Default for BitSet {
    fn default() -> Self {
        Self::new()
    }
}

impl BitAnd for BitSet {
    type Output = BitSet;

    fn bitand(self, other: Self) -> Self::Output {
        self.apply_op(&other, BitAnd::bitand)
    }
}

impl BitAnd for &BitSet {
    type Output = BitSet;

    fn bitand(self, other: Self) -> Self::Output {
        self.apply_op(other, BitAnd::bitand)
    }
}

impl BitAndAssign for BitSet {
    fn bitand_assign(&mut self, other: Self) {
        self.apply_op_assign(other, BitAndAssign::bitand_assign)
    }
}

impl BitOr for BitSet {
    type Output = BitSet;

    fn bitor(self, other: Self) -> Self::Output {
        self.apply_op(&other, BitOr::bitor)
    }
}

impl BitOr for &BitSet {
    type Output = BitSet;

    fn bitor(self, other: Self) -> Self::Output {
        self.apply_op(other, BitOr::bitor)
    }
}

impl BitOrAssign for BitSet {
    fn bitor_assign(&mut self, other: Self) {
        self.apply_op_assign(other, BitOrAssign::bitor_assign)
    }
}

impl BitXor for BitSet {
    type Output = BitSet;

    fn bitxor(self, other: Self) -> Self::Output {
        self.apply_op(&other, BitXor::bitxor)
    }
}

impl BitXor for &BitSet {
    type Output = BitSet;

    fn bitxor(self, other: Self) -> Self::Output {
        self.apply_op(other, BitXor::bitxor)
    }
}

impl BitXorAssign for BitSet {
    fn bitxor_assign(&mut self, other: Self) {
        self.apply_op_assign(other, BitXorAssign::bitxor_assign)
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
        size += uvar::from_usize(self.store.len()).ok_or(SerializingError::Overflow)?.serialize(writer)?;
        for x in self.store.iter() {
            size += x.serialize(writer)?
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += uvar::from_usize(self.store.len()).unwrap().serialized_size();
        size += self.store.len() * 0u64.serialized_size();
        size
    }
}

impl Deserialize for BitSet {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let n: uvar = Deserialize::deserialize(reader)?;
        let n = n.to_usize().ok_or(SerializingError::Overflow)?;

        let mut count = 0usize;
        let mut store: Vec<u64> = Vec::new();
        for _ in 0..n {
            let x: u64 = Deserialize::deserialize(reader)?;
            count += x.count_ones() as usize;
            store.push(x);
        }

        Ok(BitSet { store, count })
    }
}

impl fmt::Display for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[")?;
        let mut it = self.iter().peekable();
        while let Some(value) = it.next() {
            write!(f, "{}", value)?;
            if it.peek().is_some() {
                write!(f, ", ")?;
            }
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl fmt::Debug for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "BitSet({})", self.len())?;
        fmt::Display::fmt(self, f)?;
        Ok(())
    }
}

struct Bits64Iter {
    store: u64,
    idx: usize,
}

impl Bits64Iter {
    fn new(store: u64) -> Self {
        Self { store, idx: 0 }
    }
}

impl Iterator for Bits64Iter {
    type Item = bool;

    fn next(&mut self) -> Option<bool> {
        if self.idx <= 63 {
            let state = self.store & (1u64 << self.idx) != 0;
            self.idx += 1;
            Some(state)
        } else {
            None
        }
    }
}

impl FromIterator<usize> for BitSet {
    fn from_iter<T: IntoIterator<Item = usize>>(iter: T) -> Self {
        let mut bitset = BitSet::new();
        for elem in iter {
            bitset.insert(elem)
        }
        bitset
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::fmt;

    use serde::{
        de::{Deserialize, Deserializer, SeqAccess, Visitor},
        ser::{Serialize, SerializeSeq, Serializer},
    };

    use super::BitSet;

    impl Serialize for BitSet {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut seq = serializer.serialize_seq(Some(self.len()))?;

            for elem in self.iter() {
                seq.serialize_element(&elem)?;
            }

            seq.end()
        }
    }

    impl<'de> Deserialize<'de> for BitSet {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_seq(BitSetVisitor)
        }
    }

    struct BitSetVisitor;

    impl<'de> Visitor<'de> for BitSetVisitor {
        type Value = BitSet;

        fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "usize")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<BitSet, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut bitset = BitSet::new();

            while let Some(elem) = seq.next_element()? {
                bitset.insert(elem);
            }

            Ok(bitset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BitSet;
    use beserial::{Deserialize, Serialize};

    fn sample_bitset() -> BitSet {
        let mut set = BitSet::new();
        // Write 0xFEFF07
        set.remove(0);
        for i in 1..19 {
            set.insert(i);
        }
        // Set bit 70
        set.insert(70);
        set
    }

    #[test]
    fn it_can_correctly_serialize() {
        let set = sample_bitset();
        let bin = set.serialize_to_vec();
        assert_eq!(hex::decode("02000000000007FFFE0000000000000040").unwrap(), bin);
    }

    #[test]
    fn it_can_correctly_deserialize() {
        let bin = hex::decode("02000000000007FFFE0000000000000040").unwrap();
        let set = BitSet::deserialize_from_vec(&bin).unwrap();
        assert_eq!(19, set.len());
        assert_eq!(sample_bitset(), set);
    }

    #[test]
    fn it_correctly_computes_union() {
        let set1 = sample_bitset();
        let mut set2 = BitSet::new();
        set2.insert(69);
        set2.insert(70);
        let set3 = set1 & set2;
        assert_eq!(set3.len(), 1);
        {
            let mut correct = BitSet::new();
            correct.insert(70);
            assert_eq!(correct, set3);
        }
    }

    #[test]
    fn it_correctly_computes_intersection() {
        let set1 = sample_bitset();
        let mut set2 = BitSet::new();
        set2.insert(69);
        set2.insert(70);
        let set3 = set1 | set2;
        assert_eq!(set3.len(), 20);
    }
}
