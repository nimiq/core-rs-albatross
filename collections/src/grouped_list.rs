use beserial::{Serialize, Deserialize};
use itertools::Itertools;
use std::iter::{FromIterator, repeat};

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

    pub fn groups(&self) -> &Vec<Group<T>> {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.iter()
            .map(|group| group.0 as usize)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
