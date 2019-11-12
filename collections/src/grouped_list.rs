use beserial::{Serialize, Deserialize};
use itertools::Itertools;
use std::iter::{FromIterator, repeat};
use crate::compressed_list::CompressedList;
use crate::bitset::BitSet;


// GroupedList compresses a list of items by deduplication,
// internally represented by a list of length-prefixed distinct items.

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GroupedList<T>(
    #[beserial(len_type(u16))]
    pub Vec<Group<T>>
) where T: Clone + Eq + PartialEq + Serialize + Deserialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Group<T>(pub u16, pub T)
    where T: Clone + Eq + PartialEq + Serialize + Deserialize;

impl<T> FromIterator<T> for GroupedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    fn from_iter<I: IntoIterator<Item=T>>(iter: I) -> Self {
        Self(iter.into_iter()
            // Group by items
            .group_by(|item| item.clone())
            .into_iter()
            .map(|(_, mut iter)| {
                // Pop first item
                let mut count = 1u16;
                let item = iter.next().unwrap();
                // Count rest
                count += iter.count() as u16;
                Group(count, item)
            })
            .collect()
        )
    }
}

impl<T> GroupedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    pub fn empty() -> Self {
        Self(vec![])
    }

    pub fn iter<'a>(&'a self) -> impl Iterator<Item=T> + 'a {
        self.0.iter()
            .flat_map(|group| repeat(group.1.clone()).take(group.0 as usize))
    }

    pub fn iter_groups<'a>(&'a self) -> impl Iterator<Item=&Group<T>> + 'a {
        self.0.iter()
    }

    pub fn groups(&self) -> &Vec<Group<T>> {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.iter()
            .map(|group| group.0 as usize)
            .sum()
    }

    pub fn num_groups(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&Group<T>> {
        self.0.get(i)
    }

    pub fn as_compressed(&self) -> CompressedList<T> {
        let mut count = 0u16;
        let mut allocation = BitSet::new();
        let mut distinct = Vec::with_capacity(self.0.len());
        for Group(group_count, item) in self.groups().iter() {
            allocation.insert(count as usize);
            distinct.push(item.clone());
            count += *group_count;
        }
        CompressedList { count, distinct, allocation }
    }
}

impl<T> Default for GroupedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    fn default() -> Self {
        GroupedList(Vec::new())
    }
}

impl<T> From<CompressedList<T>> for GroupedList<T>
    where T: Clone + Eq + PartialEq + Serialize + Deserialize
{
    fn from(mut compressed: CompressedList<T>) -> Self {
        let mut current: Option<T> = None;
        let mut groups = Vec::with_capacity(compressed.distinct.len());
        let mut distinct = compressed.distinct.drain(..);
        let mut n = 0;

        for i in 0 .. (compressed.count as usize) {
            if compressed.allocation.contains(i) {
                if let Some(x) = current {
                    groups.push(Group(n, x))
                }
                current = distinct.next();
                n = 0;
            }
            n += 1;
        }
        if let Some(x) = current {
            groups.push(Group(n, x))
        }

        GroupedList(groups)
    }
}
