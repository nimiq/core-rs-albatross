extern crate num_traits;

use std::ops::*;

use num_traits::identities::Zero;

pub struct SegmentTree<T, U>
where
    T: Ord + Eq + Clone,
    U: Copy + Add + AddAssign + Zero + PartialOrd + PartialEq,
{
    root: Option<Box<Node<T, U>>>,
    size: usize,
}

#[derive(Debug, PartialOrd, PartialEq)]
pub struct Range<U>
where
    U: Copy + Add + AddAssign + Zero + PartialOrd + PartialEq,
{
    pub weight: U,
    pub offset: U,
}

struct Node<T, U>
where
    T: Ord + Eq + Clone,
    U: Copy + Add + AddAssign + Zero + PartialOrd + PartialEq,
{
    key: T,
    weight: U,
    children: Option<[Box<Node<T, U>>; 2]>,
}

impl<T, U> SegmentTree<T, U>
where
    T: Ord + Eq + Clone,
    U: Copy + Add + AddAssign + Zero + PartialOrd + PartialEq,
{
    /// Builds a new segment tree.
    ///
    /// weight keys must be in ascending order and must not contain duplicates
    pub fn new(weights: &mut [(T, U)]) -> Self {
        debug_assert!(weights.windows(2).all(|w| w[0].0 < w[1].0));
        SegmentTree {
            root: Self::build_tree(weights),
            size: weights.len(),
        }
    }

    fn build_tree(entries: &[(T, U)]) -> Option<Box<Node<T, U>>> {
        match entries.len() {
            0 => None,
            1 => Some(Box::new(Node {
                key: entries[0].0.clone(),
                weight: entries[0].1,
                children: None,
            })),
            _ => {
                let mid = entries.len() / 2;
                let left_slice = &entries[..mid];
                let right_slice = &entries[mid..];
                let left = Self::build_tree(left_slice).unwrap();
                let right = Self::build_tree(right_slice).unwrap();
                Some(Box::new(Node {
                    key: right_slice[0].0.clone(),
                    weight: left.weight + right.weight,
                    children: Some([left, right]),
                }))
            }
        }
    }

    // Get searches for the node with the specified key.
    // It returns the size of the node and its offset (sum of preceding nodes).
    pub fn get(&self, key: T) -> Option<Range<U>> {
        let mut node = self.root.as_ref()?;
        let mut offset: U = Zero::zero();
        // Descend while branch node
        while node.children.is_some() {
            if key < node.key {
                // Descend to left node
                let rc = &node.children.as_ref().unwrap()[0];
                node = rc;
            } else {
                // Add weight of left tree
                offset += node.children.as_ref().unwrap()[0].weight;
                // Descend to right node
                let rc = &node.children.as_ref().unwrap()[1];
                node = rc;
            }
        }
        // Leaf node reached, check if value is correct
        if key == node.key {
            Some(Range {
                weight: node.weight,
                offset,
            })
        } else {
            None
        }
    }

    // Find returns the key with the range containing the specified point in O(log n).
    pub fn find(&self, point: U) -> Option<T> {
        if point < Zero::zero() {
            return None;
        }

        let mut node = self.root.as_ref()?;
        let mut offset: U = Zero::zero();
        // Descend while branch node
        while node.children.is_some() {
            // Check if point is greater than the total range
            if point > offset + node.weight {
                return None;
            }
            let mid = offset + node.children.as_ref().unwrap()[0].weight;
            if point < mid {
                // Descend to left node
                node = &node.children.as_ref().unwrap()[0];
            } else {
                // Add weight of left tree
                offset = mid;
                // Descend to right node
                node = &node.children.as_ref().unwrap()[1];
            }
        }
        // Leaf reached, check within range
        if point < offset + node.weight {
            Some(node.key.clone())
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn range(&self) -> U {
        if let Some(ref root) = self.root {
            root.weight
        } else {
            U::zero()
        }
    }
}
