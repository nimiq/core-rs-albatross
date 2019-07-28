use beserial::{Serialize, Deserialize};
use itertools::Itertools;
use std::iter::FromIterator;

use crate::bitset::BitSet;
use crate::grouped_list::{Group, GroupedList};

// CompressedList compresses a list of items by deduplication,
// using a bit vector to track duplicates.
// Compression algorithm:
//  - Group ranges of identical items, remembering the starting index of each group
//  - Insert all distinct items into the vector
//  - Insert all starting indexes into the BitSet (setting bit at index to one)
// Decompression algorithm:
//  - Iterate over the bits in the bitset (size is policy::SLOTS)
//  - If one bit, the next item is popped off the start of the vector
//  - If zero bit, the next item is the same as the previous item

// TODO Use BitVec instead of BitSet

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompressedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    count: u16, // TODO Move to const type parameter?
    #[beserial(len_type(u16))]
    distinct: Vec<T>,
    allocation: BitSet,
}

impl<T> CompressedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    pub fn empty() -> Self {
        CompressedList {
            count: 0,
            distinct: vec![],
            allocation: BitSet::default(),
        }
    }

    pub fn verify(&self) -> bool {
        self.distinct.len() == self.allocation.len()
            && (self.allocation.contains(0) || self.count == 0)
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl<T> FromIterator<T> for CompressedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    fn from_iter<I: IntoIterator<Item=T>>(iter: I) -> Self {
        let mut count = 0u16;
        let mut allocation = BitSet::new();
        let distinct = iter.into_iter()
            // Keep track of total count
            .map(|item| { count += 1; item })
            .enumerate()
            // Group by items
            .group_by(|item| item.1.clone())
            .into_iter()
            // Save first item of inner iterator (rest is equal)
            .map(|(_, mut iter)| iter.next().unwrap())
            // Mark new item
            .map(|(idx, item)| {
                allocation.insert(idx);
                item
            })
            .collect();
        Self { count, distinct, allocation }
    }
}

impl<'a, T> IntoIterator for &'a CompressedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    type Item = &'a T;
    type IntoIter = CompressedListIterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        CompressedListIterator {
            distinct: Box::new(self.distinct.iter()),
            allocation: Box::new(self.allocation.iter_bits().take(self.count as usize)),
            last_item: None,
        }
    }
}

pub struct CompressedListIterator<'a, T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    distinct: Box<dyn Iterator<Item=&'a T> + 'a>,
    allocation: Box<dyn Iterator<Item=bool> + 'a>,
    last_item: Option<&'a T>,
}

impl<'a, T> Iterator for CompressedListIterator<'a, T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.allocation.next().and_then(|bit| {
            if bit {
                self.last_item = Some(self.distinct.next().unwrap());
            }
            self.last_item
        })
    }
}

impl<T> From<GroupedList<T>> for CompressedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    fn from(grouped: GroupedList<T>) -> Self {
        let mut count = 0u16;
        let mut allocation = BitSet::new();
        let mut distinct = Vec::with_capacity(grouped.0.len());
        for Group(group_count, item) in grouped.groups().iter() {
            allocation.insert(count as usize);
            distinct.push(item.clone());
            count += group_count;
        }
        Self { count, distinct, allocation }
    }
}
