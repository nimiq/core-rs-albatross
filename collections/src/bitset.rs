use std::{
    fmt,
    iter::{repeat, FromIterator},
    ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign},
};

use nimiq_serde::SerializedMaxSize as _;
use serde::{
    de::{Error as _, SeqAccess, Visitor},
    ser::SerializeSeq,
    Deserialize, Deserializer, Serialize, Serializer,
};

#[inline]
fn index_and_mask(value: usize) -> (usize, u64) {
    (value >> 6, 1u64 << (value & 63))
}

#[derive(Clone, Eq, PartialOrd, Ord)]
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
        let nblocks = (nbits >> 6) + usize::from(nbits.trailing_zeros() < 6);
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

        let (mut left, mut right) = (self.store.iter(), other.store.iter());
        loop {
            let x = match (left.next(), right.next()) {
                (Some(a), Some(b)) => op(*a, *b),
                (Some(a), None) => op(*a, 0),
                (None, Some(b)) => op(0, *b),
                (None, None) => break,
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

        let (mut left, mut right) = (self.store.iter_mut(), other.store.iter());
        loop {
            match (left.next(), right.next()) {
                (Some(a), Some(b)) => {
                    op(a, *b);
                    count += a.count_ones() as usize;
                }
                (Some(a), None) => {
                    let x = 0u64;
                    op(a, x);
                    count += a.count_ones() as usize;
                }
                (None, Some(b)) => {
                    let mut x = 0u64;
                    op(&mut x, *b);
                    new.push(x);
                    count += x.count_ones() as usize;
                }
                (None, None) => break,
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
    pub fn iter_excluded(&'_ self) -> impl Iterator<Item = usize> + '_ {
        self.iter_bits()
            .enumerate()
            .filter_map(|(i, one)| if one { None } else { Some(i) })
    }

    /// Iterator of included items
    pub fn iter(&'_ self) -> impl Iterator<Item = usize> + '_ {
        self.iter_bits()
            .take(self.store.len() * 64)
            .enumerate()
            .filter_map(|(i, one)| if one { Some(i) } else { None })
    }

    /// Infinite iterator of bits
    pub fn iter_bits(&'_ self) -> impl Iterator<Item = bool> + '_ {
        self.store
            .iter()
            .flat_map(|store| Bits64Iter::new(*store))
            .chain(repeat(false))
    }

    /// Test if this is a superset of `other`
    pub fn is_superset(&self, other: &Self) -> bool {
        let (mut left, mut right) = (self.store.iter(), other.store.iter());
        loop {
            let c = match (left.next(), right.next()) {
                (Some(&a), Some(&b)) => a & b == b,
                (Some(_), None) => {
                    // Since the left side can't change the result anymore, it must be a superset
                    return true;
                }
                (None, Some(&b)) => b == 0,
                (None, None) => return true,
            };
            if !c {
                return false;
            }
        }
    }

    /// Test if this is a subset of `other`
    pub fn is_subset(&self, other: &Self) -> bool {
        other.is_superset(self)
    }

    pub fn intersection_size(&self, other: &Self) -> usize {
        self.store
            .iter()
            .zip(other.store.iter())
            .map(|(a, b)| (a & b).count_ones() as usize)
            .sum()
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
        let (mut left, mut right) = (self.store.iter(), other.store.iter());
        loop {
            match (left.next(), right.next()) {
                (Some(&a), Some(&b)) => {
                    if a != b {
                        return false;
                    }
                }
                (Some(&a), None) => {
                    if a != 0 {
                        return false;
                    }
                }
                (None, Some(&b)) => {
                    if 0 != b {
                        return false;
                    }
                }
                (None, None) => return true,
            }
        }
    }
}

impl fmt::Display for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[")?;
        let mut last_value = None;
        let mut it = self.iter().peekable();
        while let Some(value) = it.next() {
            let next_value = it.peek();
            if let Some(next_value) = next_value {
                let consecutive_last = last_value.map(|lv| lv + 1 == value).unwrap_or(false);
                let consecutive_next = value + 1 == *next_value;
                if !(consecutive_last && consecutive_next) {
                    write!(f, "{value}")?;
                    if consecutive_next {
                        write!(f, "-")?;
                    } else {
                        write!(f, ", ")?;
                    }
                }
            } else {
                write!(f, "{value}")?;
            }
            last_value = Some(value);
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

impl BitSet {
    /// Maximum size in bytes for a single `BitSet` in binary serialization.
    pub const fn max_size(largest_bit_index: usize) -> usize {
        let num_u64 = (largest_bit_index + 63) / 64;
        nimiq_serde::seq_max_size(u64::MAX_SIZE, num_u64)
    }
}

impl Serialize for BitSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            let mut seq = serializer.serialize_seq(Some(self.len()))?;
            for elem in self.iter() {
                seq.serialize_element(&elem)?;
            }
            seq.end()
        } else {
            let mut seq = serializer.serialize_seq(Some(self.store.len()))?;
            for elem in &self.store {
                seq.serialize_element(elem)?;
            }
            seq.end()
        }
    }
}

impl<'de> Deserialize<'de> for BitSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            deserializer.deserialize_seq(BitSetVisitorHumanReadable)
        } else {
            deserializer.deserialize_seq(BitSetVisitor)
        }
    }
}

struct BitSetVisitorHumanReadable;

impl<'de> Visitor<'de> for BitSetVisitorHumanReadable {
    type Value = BitSet;

    fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "an array of non-duplicate unsigned integers")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<BitSet, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut result = BitSet::new();
        while let Some(elem) = seq.next_element::<usize>()? {
            if elem > 65536 {
                return Err(A::Error::custom(format!(
                    "bitset element {} too large",
                    elem,
                )));
            }
            if result.contains(elem) {
                return Err(A::Error::custom(format!(
                    "duplicate bitset element {}",
                    elem,
                )));
            }
            result.insert(elem);
        }
        Ok(result)
    }
}

struct BitSetVisitor;

impl<'de> Visitor<'de> for BitSetVisitor {
    type Value = BitSet;

    fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "an array of u64")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<BitSet, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut store: Vec<u64> = Vec::new();
        let mut count = 0usize;

        while let Some(elem) = seq.next_element::<u64>()? {
            count += elem.count_ones() as usize;
            store.push(elem);
        }

        Ok(BitSet { store, count })
    }
}

#[cfg(test)]
mod tests {
    use nimiq_serde::{Deserialize, Serialize};
    use nimiq_test_log::test;

    use super::BitSet;

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
        assert_eq!(hex::decode("02FEFF1F40").unwrap(), bin);
    }

    #[test]
    fn it_can_correctly_deserialize() {
        let bin = hex::decode("02FEFF1F40").unwrap();
        let set = BitSet::deserialize_from_vec(&bin).unwrap();
        assert_eq!(19, set.len());
        assert_eq!(sample_bitset(), set);
    }

    #[test]
    fn it_can_correctly_serialize_human_readably() {
        assert_eq!(serde_json::to_string(&BitSet::new()).unwrap(), "[]");
        assert_eq!(
            serde_json::to_string(&sample_bitset()).unwrap(),
            "[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,70]",
        );
    }

    #[test]
    fn it_can_correctly_deserialize_human_readably() {
        assert_eq!(serde_json::from_str::<BitSet>("[]").unwrap(), BitSet::new());
        assert_eq!(
            serde_json::from_str::<BitSet>("[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,70]")
                .unwrap(),
            sample_bitset(),
        );
        assert_eq!(
            serde_json::from_str::<BitSet>("[70,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18]")
                .unwrap(),
            sample_bitset(),
        );
        assert_eq!(
            serde_json::from_str::<BitSet>("[70,18,17,16,15,14,13,12,11,10,9,8,7,6,5,4,3,2,1]")
                .unwrap(),
            sample_bitset(),
        );
    }

    #[test]
    fn it_can_error_on_human_readable_deserialization() {
        assert!(serde_json::from_str::<BitSet>("1").is_err()); // invalid type
        assert!(serde_json::from_str::<BitSet>("\"1\"").is_err()); // invalid type
        assert!(serde_json::from_str::<BitSet>("[1,1]").is_err()); // duplicate element
        assert!(serde_json::from_str::<BitSet>("[1000000000]").is_err()); // too large element
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
