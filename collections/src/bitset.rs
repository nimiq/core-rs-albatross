use std::{
    fmt,
    iter::{from_fn, repeat, FromIterator},
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
    pub const fn new() -> Self {
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

    /// Iterator of included items, in ascending order.
    ///
    /// Skips zero words instead of testing each of their bits: decoded sets keep any trailing
    /// zero words they were sent with, and each one costs a peer only one byte to send.
    pub fn iter(&'_ self) -> impl Iterator<Item = usize> + '_ {
        self.store.iter().enumerate().flat_map(|(i, &word)| {
            let mut rest = word;
            from_fn(move || {
                if rest == 0 {
                    return None;
                }
                let bit = rest.trailing_zeros() as usize;
                // Clear the lowest set bit.
                rest &= rest - 1;
                Some(i * 64 + bit)
            })
        })
    }

    /// Infinite iterator of bits
    pub fn iter_bits(&'_ self) -> impl Iterator<Item = bool> + '_ {
        self.store
            .iter()
            .flat_map(|store| Bits64Iter::new(*store))
            .chain(repeat(false))
    }

    /// Test if this is a superset of or equal to `other`
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

    /// Test if this is a subset of or equal to `other`
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
        let num_u64 = largest_bit_index.div_ceil(64);
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
                return Err(A::Error::custom(
                    format!("bitset element {elem} too large",),
                ));
            }
            if result.contains(elem) {
                return Err(A::Error::custom(
                    format!("duplicate bitset element {elem}",),
                ));
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
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };

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

    /// Reference for `iter()`: tests every bit of every word, zero words included.
    fn iter_per_bit(set: &BitSet) -> Vec<usize> {
        set.iter_bits()
            .take(set.store.len() * 64)
            .enumerate()
            .filter_map(|(i, one)| one.then_some(i))
            .collect()
    }

    fn from_words(store: Vec<u64>) -> BitSet {
        let count = store.iter().map(|word| word.count_ones() as usize).sum();
        BitSet { store, count }
    }

    #[test]
    fn it_iterates_same_items_as_per_bit_walk() {
        let mut sets = vec![
            BitSet::new(),
            sample_bitset(),
            (0..512).collect::<BitSet>(),
            [0, 63, 64 * 3 + 5, 64 * 7 + 63]
                .into_iter()
                .collect::<BitSet>(),
            from_words(vec![0, 0, 1 << 63, 0, u64::MAX, 0, 0]),
            // One item past the 512 validator slots.
            (0..=512).collect::<BitSet>(),
            // The shape a peer can send: a long run of zero words, then one bit. The bit is in
            // word 65536, past the range of a u16 index.
            from_words([vec![0; 1 << 16], vec![1 << 5]].concat()),
        ];

        // Pseudo-random words (splitmix64): a third zero, a third with one bit set, a third
        // random. Each store is checked as is and followed by three zero words.
        let mut state = 0u64;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for len in 0..32 {
            let mut store: Vec<u64> = (0..len)
                .map(|_| match next() % 3 {
                    0 => 0,
                    1 => 1 << (next() % 64),
                    _ => next(),
                })
                .collect();
            sets.push(from_words(store.clone()));
            store.extend([0; 3]);
            sets.push(from_words(store));
        }

        for set in sets {
            let items: Vec<usize> = set.iter().collect();
            assert_eq!(items, iter_per_bit(&set));
            assert_eq!(items.len(), set.len());
        }
    }

    #[test]
    fn it_iterates_decoded_set_with_zero_padding() {
        // `sample_bitset()` followed by three zero words.
        let bin = hex::decode("05FEFF1F40000000").unwrap();
        let set = BitSet::deserialize_from_vec(&bin).unwrap();
        // The decoder keeps the padding, so the set encodes back to the same bytes.
        assert_eq!(set.store.len(), 5);
        assert_eq!(set.serialize_to_vec(), bin);
        assert_eq!(set, sample_bitset());
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            sample_bitset().iter().collect::<Vec<_>>(),
        );
    }

    #[test]
    fn it_skips_zero_words_when_iterating() {
        // A peer pays one byte per zero word, so `iter()` must skip zero words instead of testing
        // their 64 bits one by one. Skipping is tens of times faster, also in debug builds, so
        // four walks with `iter()` finish well before one per-bit walk. If `iter()` walked every
        // bit again, the four walks would take about four times as long. Both sides collect the
        // same way, so only the walk differs, and each keeps its fastest of five runs to filter
        // out pauses caused by other load on the machine.
        let set = from_words([vec![1], vec![0; 1 << 15]].concat());
        let mut skipping = Duration::MAX;
        let mut per_bit = Duration::MAX;
        for _ in 0..5 {
            let start = Instant::now();
            for _ in 0..4 {
                assert_eq!(black_box(&set).iter().collect::<Vec<_>>(), [0]);
            }
            skipping = skipping.min(start.elapsed());

            let start = Instant::now();
            assert_eq!(iter_per_bit(black_box(&set)), [0]);
            per_bit = per_bit.min(start.elapsed());
        }
        assert!(
            skipping < per_bit,
            "4 walks with iter() took {skipping:?}, 1 per-bit walk took {per_bit:?}",
        );
    }
}
