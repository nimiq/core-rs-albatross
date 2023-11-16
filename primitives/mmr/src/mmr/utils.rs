use super::proof::SizeProof;
use crate::{
    error::Error,
    hash::{Hash, Merge},
};

const USIZE_BITS: u32 = 0usize.count_zeros();

#[inline]
pub(crate) fn bit_length(v: usize) -> u32 {
    USIZE_BITS - v.leading_zeros()
}

/// Computes the bagging for an iterator over peak positions.
/// The peaks must be iterated in reverse.
pub(crate) fn bagging<H: Merge, I: Iterator<Item = Result<(H, usize), Error>>>(
    peaks_rev: I,
) -> Result<H, Error> {
    // Bagging
    let mut bagging_info = None;
    for item in peaks_rev {
        let (peak_hash, peak_leaves) = item?;

        bagging_info = match bagging_info {
            None => Some((peak_hash, peak_leaves)),
            Some((root_hash, root_leaves)) => {
                let sum_leaves = root_leaves + peak_leaves;
                Some((peak_hash.merge(&root_hash, sum_leaves as u64), sum_leaves))
            }
        };
    }

    let (root, _) = bagging_info.ok_or(Error::ProveInvalidLeaves)?;
    Ok(root)
}

/// Computes a size proof for an iterator over peak positions.
pub fn prove_num_leaves<
    H: Merge + Clone,
    T: Hash<H>,
    I: Iterator<Item = Result<(H, usize, Option<H>, Option<H>), Error>>,
    F: Fn(H) -> Option<T>,
>(
    peaks_rev: I,
    f: F,
) -> Result<SizeProof<H, T>, Error> {
    // Bagging
    let mut bagging_info = None;
    for item in peaks_rev {
        let (peak_hash, peak_leaves, left_child_hash, right_child_hash) = item?;

        bagging_info = match bagging_info {
            None => Some((peak_leaves, left_child_hash, right_child_hash, peak_hash)),
            Some((root_leaves, _, _, root_hash)) => {
                let sum_leaves: usize = root_leaves + peak_leaves;

                Some((
                    sum_leaves,
                    Some(peak_hash.clone()),
                    Some(root_hash.clone()),
                    peak_hash.merge(&root_hash, sum_leaves as u64),
                ))
            }
        };
    }
    match bagging_info {
        Some((size, Some(left_hash), Some(right_hash), _)) => Ok(SizeProof::MultipleElements(
            size as u64,
            left_hash,
            right_hash,
        )),
        Some((size, None, None, hash)) if size == 1 => Ok(SizeProof::SingleElement(
            size as u64,
            f(hash).ok_or(Error::InconsistentStore)?,
        )),
        None => Ok(SizeProof::EmptyTree),
        _ => Err(Error::ProveInvalidLeaves),
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use nimiq_test_log::test;

    use super::*;
    use crate::hash::{Hash, Merge};

    pub(crate) fn hash_perfect_tree<H: Merge, T: Hash<H>>(values: &[T]) -> Option<H> {
        let len = values.len();
        if len == 0 {
            return Some(H::empty(0));
        }

        if len.count_ones() != 1 {
            return None;
        }

        if len == 1 {
            return Some(values[0].hash(1));
        }

        let mid = len >> 1;
        Some(H::merge(
            &hash_perfect_tree(&values[..mid])?,
            &hash_perfect_tree(&values[mid..])?,
            len as u64,
        ))
    }

    pub(crate) fn hash_mmr<H: Merge, T: Hash<H>>(values: &[T]) -> H {
        let mut peaks = vec![];
        let mut i = 0;
        while i < values.len() {
            // Find max sized perfect binary tree.
            let max_height = bit_length(values.len() - i) as usize - 1;
            let max_leaves = 1 << max_height;

            // Hash perfect binary tree.
            let root = hash_perfect_tree(&values[i..i + max_leaves]).unwrap();
            peaks.push(Ok((root, max_leaves)));

            i += max_leaves;
        }

        bagging(peaks.into_iter().rev()).unwrap()
    }

    #[derive(Debug, Clone, Eq, PartialEq)]
    pub(crate) struct TestHash(pub(crate) usize);
    impl Merge for TestHash {
        fn empty(prefix: u64) -> Self {
            TestHash(prefix as usize)
        }

        fn merge(&self, other: &Self, prefix: u64) -> Self {
            TestHash(self.0 * 2 + other.0 * 3 + prefix as usize)
        }
    }

    impl Hash<TestHash> for usize {
        fn hash(&self, prefix: u64) -> TestHash {
            TestHash(self * 2 + prefix as usize)
        }
    }

    #[test]
    fn test_utils_hash_correctly() {
        // hash_perfect_tree
        let values = vec![1, 3, 5, 7];
        assert_eq!(hash_perfect_tree(&values[..0]), Some(TestHash(0)));
        assert_eq!(hash_perfect_tree(&values[..3]), None);
        // 3
        assert_eq!(hash_perfect_tree(&values[..1]), Some(TestHash(3)));
        // 7
        assert_eq!(hash_perfect_tree(&values[1..2]), Some(TestHash(7)));
        // 11
        assert_eq!(hash_perfect_tree(&values[2..3]), Some(TestHash(11)));
        // 15
        assert_eq!(hash_perfect_tree(&values[3..]), Some(TestHash(15)));
        // 3 * 2 + 7 * 3 + 2 = 29
        assert_eq!(hash_perfect_tree(&values[..2]), Some(TestHash(29)));
        // 11 * 2 + 15 * 3 + 2 = 69
        assert_eq!(hash_perfect_tree(&values[2..]), Some(TestHash(69)));
        // 29 * 2 + 69 * 3 + 4 = 69
        assert_eq!(hash_perfect_tree(&values), Some(TestHash(269)));

        // hash_mmr
        // 3
        assert_eq!(hash_mmr(&values[..1]), TestHash(3));
        // 7
        assert_eq!(hash_mmr(&values[1..2]), TestHash(7));
        // 11
        assert_eq!(hash_mmr(&values[2..3]), TestHash(11));
        // 15
        assert_eq!(hash_mmr(&values[3..]), TestHash(15));
        // 3 * 2 + 7 * 3 + 2 = 29
        assert_eq!(hash_mmr(&values[..2]), TestHash(29));
        // 11 * 2 + 15 * 3 + 2 = 69
        assert_eq!(hash_mmr(&values[2..]), TestHash(69));
        // 29 * 2 + 69 * 3 + 4 = 69
        assert_eq!(hash_mmr(&values), TestHash(269));

        // Non-perfect size with bagging:
        // 1st tree: 29, 2nd tree: 11
        // 29 * 2 + 11 * 3 + 3 = 94
        assert_eq!(hash_mmr(&values[..3]), TestHash(94));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmr::utils::test_utils::TestHash;

    #[test]
    fn it_correctly_compute_bit_length() {
        assert_eq!(bit_length(0), 0);
        assert_eq!(bit_length(1), 1);
        assert_eq!(bit_length(2), 2);
        assert_eq!(bit_length(3), 2);
        assert_eq!(bit_length(255), 8);
        assert_eq!(bit_length(256), 9);
    }

    #[test]
    fn it_correctly_performs_bagging() {
        // No peaks.
        assert!(bagging::<TestHash, _>(vec![].into_iter()).is_err());

        // 1 peak.
        let mut positions = vec![Ok((TestHash(2), 2))];
        assert_eq!(
            bagging(positions.clone().into_iter().rev()),
            Ok(TestHash(2))
        );
        assert_eq!(bagging(positions.clone().into_iter()), Ok(TestHash(2)));

        // 2 peaks.
        positions.push(Ok((TestHash(1), 1)));
        assert_eq!(
            bagging(positions.clone().into_iter().rev()),
            Ok(TestHash(10))
        );
        assert_eq!(bagging(positions.clone().into_iter()), Ok(TestHash(11)));

        // 3 peaks.
        positions.push(Ok((TestHash(2), 2)));
        assert_eq!(
            bagging(positions.clone().into_iter().rev()),
            Ok(TestHash(42))
        );
        assert_eq!(bagging(positions.into_iter()), Ok(TestHash(42)));
    }
}
