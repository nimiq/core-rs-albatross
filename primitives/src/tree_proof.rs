use std::io::Write;

use byteorder::WriteBytesExt;
use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher, SerializeContent};

/// Stores a complete, balanced binary tree of hashes, starting from the root at index `0`.
///
/// The children of node `i` are at `2 * i + 1` and `2 * i + 2`.
/// The parent of node `i` is at `(i - 1) / 2`.
/// All of the leaf nodes are in at the same level
#[derive(Default)]
pub struct TreeProof {
    heap: Vec<Blake2bHash>,
}

fn hash_empty() -> Blake2bHash {
    let mut hasher = Blake2bHasher::new();
    hasher.write_u8(0).unwrap();
    hasher.finish()
}

fn hash_branch(left: &Blake2bHash, right: &Blake2bHash) -> Blake2bHash {
    let mut hasher = Blake2bHasher::new();
    hasher.write_u8(1).unwrap();
    hasher.write_all(left.as_bytes()).unwrap();
    hasher.write_all(right.as_bytes()).unwrap();
    hasher.finish()
}

fn hash_leaf<T: SerializeContent>(item: &T) -> Blake2bHash {
    let mut hasher = Blake2bHasher::new();
    hasher.write_u8(2).unwrap();
    item.serialize_content::<_, Blake2bHash>(&mut hasher)
        .unwrap();
    hasher.finish()
}

impl TreeProof {
    pub fn empty() -> TreeProof {
        TreeProof::default()
    }
    pub fn new<T, I>(iter: I) -> TreeProof
    where
        I: IntoIterator<Item = T>,
        T: SerializeContent,
    {
        TreeProof::from_hashes(iter.into_iter().map(|item| hash_leaf(&item)).collect())
    }
    fn from_hashes(hashes: Vec<Blake2bHash>) -> TreeProof {
        if hashes.is_empty() {
            return TreeProof::empty();
        }
        let number_of_leaves_perfect_tree = hashes.len().next_power_of_two();
        let number_of_leaves_on_last_level = hashes.len() * 2 - number_of_leaves_perfect_tree;
        let expected_size = number_of_leaves_perfect_tree - 1 + number_of_leaves_on_last_level;

        let mut heap = hashes;
        heap.reserve_exact(expected_size - heap.len());
        heap.rotate_left(number_of_leaves_on_last_level);
        heap.reverse();
        let mut nodes = 0..number_of_leaves_on_last_level;
        while nodes.len() > 1 {
            for idx in nodes.clone().step_by(2) {
                heap.push(hash_branch(&heap[idx + 1], &heap[idx]));
            }
            nodes = nodes.end..heap.len();
        }
        heap.reverse();
        assert!(heap.len() == expected_size);
        TreeProof { heap }
    }

    pub fn root_hash(&self) -> Blake2bHash {
        self.heap.first().cloned().unwrap_or_else(hash_empty)
    }
}

#[cfg(test)]
mod test {
    use nimiq_test_log::test;

    use super::{hash_branch, hash_empty, hash_leaf, TreeProof};

    #[test]
    fn empty() {
        // no tree
        assert_eq!(TreeProof::new::<&&str, _>(&[]).root_hash(), hash_empty());
    }
    #[test]
    fn one() {
        // 1
        assert_eq!(TreeProof::new(["1"]).root_hash(), hash_leaf(&"1"));
    }
    #[test]
    fn two() {
        //   *
        //  / \
        // 1   2
        assert_eq!(
            TreeProof::new(["1", "2"]).root_hash(),
            hash_branch(&hash_leaf(&"1"), &hash_leaf(&"2")),
        );
    }
    #[test]
    fn three() {
        //     *
        //    / \
        //   *   3
        //  / \
        // 1   2
        assert_eq!(
            TreeProof::new(["1", "2", "3"]).root_hash(),
            hash_branch(
                &hash_branch(&hash_leaf(&"1"), &hash_leaf(&"2")),
                &hash_leaf(&"3"),
            ),
        );
    }
    #[test]
    fn four() {
        //      *
        //    /   \
        //   *     *
        //  / \   / \
        // 1   2 3   4
        assert_eq!(
            TreeProof::new(["1", "2", "3", "4"]).root_hash(),
            hash_branch(
                &hash_branch(&hash_leaf(&"1"), &hash_leaf(&"2")),
                &hash_branch(&hash_leaf(&"3"), &hash_leaf(&"4")),
            ),
        );
    }
    #[test]
    fn five() {
        //        *
        //      /   \
        //     *     *
        //    / \   / \
        //   *   3 4   5
        //  / \
        // 1   2
        assert_eq!(
            TreeProof::new(["1", "2", "3", "4", "5"]).root_hash(),
            hash_branch(
                &hash_branch(
                    &hash_branch(&hash_leaf(&"1"), &hash_leaf(&"2")),
                    &hash_leaf(&"3"),
                ),
                &hash_branch(&hash_leaf(&"4"), &hash_leaf(&"5")),
            ),
        );
    }
}
