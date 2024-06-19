use std::cmp::Ordering;

use crate::mmr::utils::bit_length;

/// Structure to hold a node's position.
/// This structure contains and caches additional information that normally would need to be computed
/// from the index.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Position {
    pub(crate) index: usize,
    pub(crate) height: usize,
    pub(crate) right_node: bool,
}

// We have some convenience functions that are not used yet.
#[allow(dead_code)]
impl Position {
    pub(crate) fn sibling(&self) -> Position {
        let sibling_offset = (2 << self.height) - 1; // 2^(h+1) - 1
        Position {
            index: if self.right_node {
                self.index - sibling_offset
            } else {
                self.index + sibling_offset
            },
            height: self.height,
            right_node: !self.right_node,
        }
    }

    pub(crate) fn parent(&self) -> Position {
        let parent_offset = 2 << self.height; // 2^(h+1) - 1 + 1
        let index = if self.right_node {
            self.index + 1
        } else {
            self.index + parent_offset
        };
        Position {
            index,
            height: self.height + 1,
            right_node: index_to_height(index + 1) > self.height + 1,
        }
    }

    pub(crate) fn left_child(&self) -> Option<Position> {
        let parent_offset = 1 << self.height; // 2^h - 1 + 1
        if self.height > 0 {
            return Some(Position {
                index: self.index - parent_offset,
                height: self.height - 1,
                right_node: false,
            });
        }
        None
    }

    pub(crate) fn right_child(&self) -> Option<Position> {
        if self.height > 0 {
            return Some(Position {
                index: self.index - 1,
                height: self.height - 1,
                right_node: true,
            });
        }
        None
    }

    pub(crate) fn leftmost_leaf(&self) -> Position {
        if self.height == 0 {
            return *self;
        }

        let leftmost_leaf_offset = (2 << self.height) - 2; // 2^(h+1) - 2
        Position {
            index: self.index - leftmost_leaf_offset,
            height: 0,
            right_node: false,
        }
    }

    pub(crate) fn rightmost_leaf(&self) -> Position {
        if self.height == 0 {
            return *self;
        }

        let rightmost_leaf_offset = self.height;
        Position {
            index: self.index - rightmost_leaf_offset,
            height: 0,
            right_node: true,
        }
    }

    pub(crate) fn num_leaves(&self) -> usize {
        1 << self.height // A perfect binary tree has 2^h leaves.
    }
}

impl From<usize> for Position {
    fn from(index: usize) -> Self {
        let height = index_to_height(index);
        let next_height = index_to_height(index + 1);
        Position {
            index,
            height,
            right_node: next_height > height,
        }
    }
}

impl From<Position> for usize {
    fn from(pos: Position) -> Self {
        pos.index
    }
}

impl<'a> From<&'a Position> for usize {
    fn from(pos: &'a Position) -> Self {
        pos.index
    }
}

impl PartialEq for Position {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl Eq for Position {}

impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Position {
    fn cmp(&self, other: &Self) -> Ordering {
        self.index.cmp(&other.index)
    }
}

/// Calculate whether a number only consists of 1-bits (except the leading zeros).
/// That means 00000111 is considered all-ones but 00001110 is not.
fn all_ones(num: usize) -> bool {
    if num == 0 {
        return false;
    }
    // Find the highest set bit
    let highest_set_bit = usize::BITS - num.leading_zeros();
    // Create a mask with all 1s from the highest set bit to the least significant bit
    let mask = (1 << highest_set_bit) - 1;
    // Check if the number matches the mask
    num == mask
}

/// In order to move left on the same level, we subtract the index by its most significant bit
/// minus 1 (note that this is not necessarily the *next* node on the left on the same level).
/// Important: This uses 1-based indexing.
fn move_left(index: usize) -> usize {
    let bit_length = bit_length(index);
    let most_significant_bits = 1 << (bit_length - 1); // 2^(l - 1)
    index - (most_significant_bits as usize - 1)
}

/// To calculate the height for any index, we move left in the tree (on the same height)
/// as long as we can. Then, the height is the number of 1 bits of the resulting index minus 1
/// assuming our indices start at 1.
///
/// In order to move left on the same level, we subtract the index by its most significant bit
/// minus 1.
///
/// The input is a 0-based index and all conversion is done within the function.
pub(crate) fn index_to_height(mut index: usize) -> usize {
    index += 1; // 1-based indexing.

    // Move left as long as possible.
    while !all_ones(index) {
        index = move_left(index)
    }

    index.count_ones() as usize - 1
}

/// Calculates the index inside the MMR store for the `leaf_number`th leaf.
/// Both numbers are 0-based.
pub fn leaf_number_to_index(leaf_number: usize) -> usize {
    2 * leaf_number - leaf_number.count_ones() as usize
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn it_correctly_computes_positions() {
        // Start position.
        let mut pos = Position::from(0);

        assert!(pos.left_child().is_none());
        assert!(pos.right_child().is_none());

        // A tuple of operation, resulting index, height, and the right_node bit.
        enum Op {
            Parent,
            Sibling,
            LeftChild,
            RightChild,
            LeftLeaf,
            RightLeaf,
            Set,
        }
        let ops = vec![
            (Op::Parent, 2, 1, false),
            (Op::Sibling, 5, 1, true),
            (Op::Parent, 6, 2, false),
            (Op::RightChild, 5, 1, true),
            (Op::RightChild, 4, 0, true),
            (Op::LeftLeaf, 4, 0, true),
            (Op::RightLeaf, 4, 0, true),
            (Op::Parent, 5, 1, true),
            (Op::Parent, 6, 2, false),
            (Op::Parent, 14, 3, false),
            (Op::RightLeaf, 11, 0, true),
            (Op::Set, 14, 3, false),
            (Op::LeftLeaf, 0, 0, false),
            (Op::Set, 11, 0, true),
            (Op::Sibling, 10, 0, false),
            (Op::Parent, 12, 1, true),
            (Op::LeftChild, 10, 0, false),
        ];
        for (j, (op, index, height, right_node)) in ops.into_iter().enumerate() {
            pos = match op {
                Op::Parent => pos.parent(),
                Op::Sibling => pos.sibling(),
                Op::LeftChild => pos.left_child().unwrap(),
                Op::RightChild => pos.right_child().unwrap(),
                Op::LeftLeaf => pos.leftmost_leaf(),
                Op::RightLeaf => pos.rightmost_leaf(),
                Op::Set => Position::from(index),
            };

            assert_eq!(pos.index, index, "op {j}");
            assert_eq!(pos.height, height, "wrong height @ index {index}, op {j}");
            assert_eq!(
                pos.right_node, right_node,
                "wrong right_node @ index {index}, op {j}"
            );

            assert_eq!(pos.left_child().is_none(), pos.height == 0);
            assert_eq!(pos.right_child().is_none(), pos.height == 0);
        }
    }

    #[test]
    fn it_correctly_computes_number_of_leaves() {
        let leaves = vec![
            (0, 1),
            (1, 1),
            (3, 1),
            (4, 1),
            (7, 1),
            (2, 2),
            (5, 2),
            (9, 2),
            (6, 4),
            (13, 4),
        ];
        for (index, num_leaves) in leaves {
            assert_eq!(Position::from(index).num_leaves(), num_leaves);
        }
    }

    #[test]
    fn it_correctly_checks_for_ones() {
        let num_bits = 8;

        // Fill with all values that have only 1 bits up to `num_bits`.
        let mut ones = HashSet::new();
        let mut v = 0;
        for _ in 0..num_bits {
            v = (v << 1) | 1;
            ones.insert(v);
        }

        let num_values = 1 << num_bits;
        for i in 0..num_values {
            assert_eq!(all_ones(i), ones.contains(&i));
        }
    }

    #[test]
    fn it_correctly_moves_left() {
        let pairs = vec![
            (1, 0),
            (3, 0),
            (4, 1),
            (7, 0),
            (15, 0),
            (5, 2),
            (9, 2),
            (13, 6),
        ];
        for (right, left) in pairs {
            assert_eq!(move_left(right + 1), left + 1, "move left from {right}");
        }
    }

    #[test]
    fn it_correctly_computes_height() {
        let heights = [
            0, 0, 1, 0, 0, 1, 2, 0, 0, 1, 0, 0, 1, 2, 3, 0, 0, 1, 0, 0, 1, 2,
        ];
        for i in 0..heights.len() {
            assert_eq!(index_to_height(i), heights[i]);
        }
    }

    #[test]
    fn it_correctly_computes_indices() {
        let leaf_indices = [0, 1, 3, 4, 7, 8, 10, 11, 15, 16, 18, 19];
        for i in 0..leaf_indices.len() {
            assert_eq!(leaf_number_to_index(i), leaf_indices[i]);
        }
    }
}
