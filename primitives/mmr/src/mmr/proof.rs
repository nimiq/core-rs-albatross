use std::collections::VecDeque;

use crate::{
    error::Error,
    hash::{Hash, Merge},
    mmr::{
        peaks::PeakIterator,
        position::{leaf_number_to_index, Position},
        utils::bagging,
    },
};

#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde-derive",
    derive(nimiq_serde::Serialize, nimiq_serde::Deserialize)
)]
pub enum SizeProof<H: Merge, T: Hash<H>> {
    EmptyTree,
    SingleElement(u64, T),
    MultipleElements(u64, H, H),
}

impl<H: Merge + Eq, T: Hash<H>> SizeProof<H, T> {
    pub fn verify(&self, hash: &H) -> bool {
        let self_hash = match self {
            SizeProof::EmptyTree => H::empty(0),
            SizeProof::SingleElement(size, item) => item.hash(*size),
            SizeProof::MultipleElements(size, left, right) => left.merge(right, *size),
        };
        hash == &self_hash
    }

    pub fn size(&self) -> u64 {
        match self {
            SizeProof::EmptyTree => 0,
            SizeProof::SingleElement(size, _) | SizeProof::MultipleElements(size, _, _) => *size,
        }
    }
}

/// A Merkle proof for a MMR.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde-derive",
    derive(nimiq_serde::Serialize, nimiq_serde::Deserialize)
)]
pub struct Proof<H> {
    pub mmr_size: usize,
    pub nodes: Vec<H>,
}

impl<H: Merge> Proof<H> {
    pub(crate) fn new(mmr_size: usize) -> Self {
        Proof {
            mmr_size,
            nodes: vec![],
        }
    }
}

impl<H: Merge + Clone> Proof<H> {
    /// `leaves` is a slice of tuples, where the first position is the index of the leaf
    /// and the second position is the un-hashed item.
    pub fn calculate_root<T>(&self, leaves: &[(usize, &T)]) -> Result<H, Error>
    where
        T: Hash<H>,
    {
        // Sort by positions and check that we only prove leaves.
        let positions: Result<Vec<(Position, H)>, Error> = leaves
            .iter()
            .map(|(index, value)| {
                let pos = Position::from(leaf_number_to_index(*index));
                // Make sure that we only ever prove leaf nodes.
                if pos.index >= self.mmr_size {
                    return Err(Error::ProveInvalidLeaves);
                }
                let hash = value.hash(pos.num_leaves() as u64);
                Ok((pos, hash))
            })
            .collect();
        let mut positions = positions?;
        positions.sort_unstable_by_key(|(pos, _)| *pos);

        let peaks = self.calculate_peak_roots(&positions)?;
        bagging(peaks.into_iter().map(Ok).rev())
    }

    /// Leaves are expected to be sorted by index.
    fn calculate_peak_roots(&self, leaves: &[(Position, H)]) -> Result<Vec<(H, usize)>, Error> {
        // Shortcut for a tree of size 1.
        if self.mmr_size == 1 && leaves.len() == 1 && leaves[0].0.index == 0 {
            return Ok(vec![(leaves[0].1.clone(), 1)]);
        }

        let mut proof_iter = self.nodes.iter();

        let mut peaks_hashes = vec![];
        let mut prev_i = 0; // Index into the `leaves` slice.
        for peak in PeakIterator::new(self.mmr_size) {
            // Find all positions belonging to this peak.
            let peak_positions = &leaves[prev_i..];
            let i_offset = peak_positions
                .iter()
                .position(|(pos, _)| pos > &peak)
                .unwrap_or(peak_positions.len());

            // If there is no leaf node, we need to take a proof node for this peak.
            if i_offset == 0 {
                if let Some(hash) = proof_iter.next() {
                    peaks_hashes.push((hash.clone(), peak.num_leaves()));
                } else {
                    // If there's no proof node left, it might be that it was part of
                    // a bagged proof node before.
                    // We then need to add up the number of leaves.
                    if let Some((_, num_leaves)) = peaks_hashes.last_mut() {
                        *num_leaves += peak.num_leaves();
                    } else {
                        break;
                    }
                }
            } else {
                // Calculate peak hash from nodes instead.
                peaks_hashes.push((
                    Self::calculate_root_for_peak(
                        peak,
                        &peak_positions[..i_offset],
                        &mut proof_iter,
                    )?,
                    peak.num_leaves(),
                ));
            }

            prev_i += i_offset;
        }

        // Check if all leaves have been used and make sure there are no proof nodes left.
        if prev_i != leaves.len() || proof_iter.next().is_some() {
            return Err(Error::InvalidProof);
        }

        Ok(peaks_hashes)
    }

    fn calculate_root_for_peak<'a, I>(
        peak_position: Position,
        leaves: &[(Position, H)],
        proof_iter: &mut I,
    ) -> Result<H, Error>
    where
        H: 'a,
        I: Iterator<Item = &'a H>,
    {
        // Shortcut if there's only one position, which coincides with the peak:
        // In that case, the leaf hash is actually the peak hash.
        if leaves.len() == 1 && leaves[0].0 == peak_position {
            return Ok(leaves[0].1.clone());
        }

        // If the shortcut does not apply, we start with the positions in the list as a queue
        // of positions to look at.
        // We can think of this queue as a queue of elements that we can compute from
        // the data we have (including leaf nodes and proof nodes).
        // We iterate over this queue and move up the tree.
        let mut queue: VecDeque<_> = leaves.iter().cloned().collect();
        while let Some((position, hash)) = queue.pop_front() {
            // Stop once we reached the peak.
            if position == peak_position {
                return Ok(hash);
            }

            // Otherwise, check if we need to take a proof node on this level.
            // Following the same process as for constructing the proof, we can check whether
            // the node required is at the front of the queue. If not, we need to take a proof
            // node instead.
            let sibling = position.sibling();
            let sibling_hash = if queue.front().map(|(pos, _)| pos) == Some(&sibling) {
                // If the sibling is part of the queue, take its hash.
                queue.pop_front().unwrap().1
            } else {
                // If the sibling is not part of the queue, take a proof node.
                proof_iter.next().ok_or(Error::InvalidProof)?.clone()
            };

            let parent_pos = position.parent();
            let parent_hash = if position.right_node {
                sibling_hash.merge(&hash, parent_pos.num_leaves() as u64)
            } else {
                hash.merge(&sibling_hash, parent_pos.num_leaves() as u64)
            };

            // Add the parent to the queue.
            queue.push_back((parent_pos, parent_hash));
        }
        Err(Error::InvalidProof)
    }
}

impl<H: Merge + Clone + Eq> Proof<H> {
    /// Verifies a Merkle proof given the root of the MMR and the leaves that are to be proven.
    pub fn verify<T>(&self, root: &H, leaves: &[(usize, &T)]) -> Result<bool, Error>
    where
        T: Hash<H>,
    {
        // If the proof came from an empty tree we just need to check that we are not trying to
        // prove any leaves and that the given root matches the empty tree root.
        if self.mmr_size == 0 && self.nodes.is_empty() {
            return Ok(root == &H::empty(0));
        }

        // Otherwise we just calculate the root.
        self.calculate_root(leaves)
            .map(|calculated_root| &calculated_root == root)
    }
}

/// A Merkle proof for a MMR. This is equal to the regular Merkle proof, but has the `assume_previous`
/// flag which can be used when we are verifying consecutive range proofs.
#[cfg_attr(
    feature = "serde-derive",
    derive(nimiq_serde::Serialize, nimiq_serde::Deserialize)
)]
pub struct RangeProof<H> {
    pub proof: Proof<H>,
    pub assume_previous: bool,
}

impl<H: Merge + Clone> RangeProof<H> {
    /// `leaves` is a slice of tuples, where the first position is the index of the leaf
    /// and the second position is the un-hashed item.
    pub fn calculate_root<T>(&self, leaves: &[(usize, &T)]) -> Result<H, Error>
    where
        T: Hash<H>,
    {
        if self.assume_previous {
            return Err(Error::IncompleteProof);
        }
        self.proof.calculate_root(leaves)
    }

    /// `leaf_index` is the number of leaves preceding this proof.
    pub fn calculate_root_with_start<T>(&self, leaf_index: usize, leaves: &[T]) -> Result<H, Error>
    where
        T: Hash<H>,
    {
        if self.assume_previous {
            return Err(Error::IncompleteProof);
        }

        let leaves = self.convert_leaves_with_start(leaf_index, leaves)?;
        self.proof.calculate_root(&leaves)
    }

    #[inline]
    fn convert_leaves_with_start<'a, T>(
        &self,
        leaf_index: usize,
        leaves: &'a [T],
    ) -> Result<Vec<(usize, &'a T)>, Error>
    where
        T: Hash<H>,
    {
        if !leaves.is_empty()
            && leaf_number_to_index(leaf_index + leaves.len() - 1) >= self.proof.mmr_size
        {
            return Err(Error::ProveInvalidLeaves);
        }

        Ok(leaves
            .iter()
            .enumerate()
            .map(|(i, t)| (leaf_index + i, t))
            .collect())
    }
}

impl<H: Merge + Clone + Eq> RangeProof<H> {
    /// `leaves` is a slice of tuples, where the first position is the index
    /// and the second position is the un-hashed item.
    pub fn verify<T>(&self, root: &H, leaves: &[(usize, &T)]) -> Result<bool, Error>
    where
        T: Hash<H>,
    {
        if self.assume_previous {
            return Err(Error::IncompleteProof);
        }
        self.proof.verify(root, leaves)
    }

    /// `leaf_index` is the number of leaves preceding this proof.
    pub fn verify_with_start<T>(
        &self,
        root: &H,
        leaf_index: usize,
        leaves: &[T],
    ) -> Result<bool, Error>
    where
        T: Hash<H>,
    {
        if self.assume_previous {
            return Err(Error::IncompleteProof);
        }

        let leaves = self.convert_leaves_with_start(leaf_index, leaves)?;
        self.proof.verify(root, &leaves)
    }
}

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;

    use nimiq_test_log::test;

    use super::*;
    use crate::{
        error::Error,
        mmr::{utils::test_utils::TestHash, MerkleMountainRange},
        store::{memory::MemoryStore, Store},
    };

    #[test]
    fn it_correctly_constructs_proofs() {
        #[derive(Debug)]
        enum Node {
            Store(usize),
            Hash(TestHash),
        }
        /// Proves the values from `leaves` at indices `to_prove` (into the `leaves`).
        /// Then checks proof against `proof_indices` included (into the tree).
        fn test_proof<S: Store<TestHash>>(
            mmr: &MerkleMountainRange<TestHash, S>,
            leaves: &[usize],
            to_prove: &[usize],
            proof_nodes: &[Node],
        ) {
            let proof = mmr.prove(to_prove, None).unwrap();
            assert_eq!(proof.mmr_size, mmr.len());
            assert_eq!(proof.nodes.len(), proof_nodes.len());

            for (i, node) in proof_nodes.iter().enumerate() {
                assert_eq!(
                    proof.nodes[i],
                    match node {
                        Node::Store(index) => mmr.store.get(*index).unwrap(),
                        Node::Hash(h) => h.clone(),
                    },
                    "Proof node {i} should be {node:?}"
                );
            }

            let tree_leaves: Vec<_> = to_prove.iter().map(|&i| (i, &leaves[i])).collect();
            assert_eq!(
                proof.verify(&mmr.get_root().unwrap(), &tree_leaves),
                Ok(true)
            );
        }

        let nodes = vec![2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        for &v in nodes[..2].iter() {
            mmr.push(&v).unwrap();
        }

        /*      2
         *    /   \
         *   0     1
         */
        test_proof(&mmr, &nodes, &[0], &[Node::Store(1)]);
        test_proof(&mmr, &nodes, &[1], &[Node::Store(0)]);
        test_proof(&mmr, &nodes, &[1, 0], &[]);

        assert_eq!(
            mmr.prove(&[], None).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove(&[4], None).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove(&[2], None).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );

        /*        root
         *       /    \
         *      2      \
         *    /   \     \
         *   0     1     3
         *   0     1     2
         */
        mmr.push(&nodes[2]).unwrap();

        test_proof(&mmr, &nodes, &[0], &[Node::Store(1), Node::Store(3)]);
        test_proof(&mmr, &nodes, &[2], &[Node::Store(2)]);

        /*
         *                      14
         *            /                    \
         *           6                      13
         *      /         \             /         \
         *     2           5           9           12           17
         *   /   \       /   \       /   \       /    \       /    \
         *  0     1     3     4     7     8     10    11     15    16    18
         *  0     1     2     3     4     5     6     7      8     9     10
         */
        for &v in nodes[3..].iter() {
            mmr.push(&v).unwrap();
        }

        let bagged_rhs = bagging(
            vec![
                Ok((mmr.store.get(18).unwrap(), 1)),
                Ok((mmr.store.get(17).unwrap(), 2)),
            ]
            .into_iter(),
        )
        .unwrap();

        test_proof(
            &mmr,
            &nodes,
            &[1],
            &[
                Node::Store(0),
                Node::Store(5),
                Node::Store(13),
                Node::Hash(bagged_rhs.clone()),
            ],
        );

        test_proof(
            &mmr,
            &nodes,
            &[1, 2, 3, 6],
            &[
                Node::Store(0),
                Node::Store(11),
                Node::Store(9),
                Node::Hash(bagged_rhs),
            ],
        );

        test_proof(
            &mmr,
            &nodes,
            &[0, 9],
            &[
                Node::Store(1),
                Node::Store(5),
                Node::Store(13),
                Node::Store(15),
                Node::Store(18),
            ],
        );

        test_proof(
            &mmr,
            &nodes,
            &[0, 9],
            &[
                Node::Store(1),
                Node::Store(5),
                Node::Store(13),
                Node::Store(15),
                Node::Store(18),
            ],
        );

        test_proof(
            &mmr,
            &nodes,
            &[9],
            &[Node::Store(14), Node::Store(15), Node::Store(18)],
        );

        test_proof(&mmr, &nodes, &[10], &[Node::Store(14), Node::Store(17)]);
    }

    #[test]
    fn it_correctly_discards_invalid_proofs() {
        let nodes = vec![2, 3];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        for &v in nodes.iter() {
            mmr.push(&v).unwrap();
        }

        /*      2
         *    /   \
         *   0     1
         */
        let proof = mmr.prove(&[1], None).unwrap();
        // Proof for wrong position.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(0, &2)]),
            Ok(false)
        );
        // Proof for wrong value.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(1, &2)]),
            Ok(false)
        );

        // Proof for less values.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(0, &2), (1, &3)]),
            Err(Error::InvalidProof)
        );

        // Proof for non-leaves.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(2, &0)]),
            Err(Error::ProveInvalidLeaves)
        );
    }

    #[test]
    fn it_correctly_constructs_range_proofs() {
        #[derive(Debug)]
        enum Node {
            Store(usize),
            Hash(TestHash),
        }
        /// Proves the values from `leaves` at indices `to_prove` (into the `leaves`).
        /// Then checks proof against `proof_indices` included (into the tree).
        fn test_proof<S: Store<TestHash>>(
            mmr: &MerkleMountainRange<TestHash, S>,
            leaves: &[usize],
            to_prove: RangeInclusive<usize>,
            proof_nodes: &[Node],
            assume_previous: bool,
        ) {
            let proof = mmr
                .prove_range(to_prove.clone(), Some(mmr.len()), assume_previous)
                .unwrap();
            assert_eq!(proof.assume_previous, assume_previous);
            assert_eq!(proof.proof.mmr_size, mmr.len());
            assert_eq!(proof.proof.nodes.len(), proof_nodes.len());

            for (i, node) in proof_nodes.iter().enumerate() {
                assert_eq!(
                    proof.proof.nodes[i],
                    match node {
                        Node::Store(index) => mmr.store.get(*index).unwrap(),
                        Node::Hash(h) => h.clone(),
                    },
                    "Proof node {i} should be {node:?}"
                );
            }

            let tree_leaves: Vec<_> = to_prove.clone().map(|i| (i, &leaves[i])).collect();
            assert_eq!(
                proof.verify(&mmr.get_root().unwrap(), &tree_leaves),
                if assume_previous {
                    Err(Error::IncompleteProof)
                } else {
                    Ok(true)
                }
            );

            assert_eq!(
                proof.verify_with_start(
                    &mmr.get_root().unwrap(),
                    *to_prove.start(),
                    &leaves[to_prove],
                ),
                if assume_previous {
                    Err(Error::IncompleteProof)
                } else {
                    Ok(true)
                }
            );
        }

        let nodes = vec![2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        for &v in nodes[..2].iter() {
            mmr.push(&v).unwrap();
        }

        /*      2
         *    /   \
         *   0     1
         */
        test_proof(&mmr, &nodes, 0..=0, &[Node::Store(1)], false);
        test_proof(&mmr, &nodes, 0..=0, &[Node::Store(1)], true);
        test_proof(&mmr, &nodes, 1..=1, &[Node::Store(0)], false);
        test_proof(&mmr, &nodes, 1..=1, &[], true);
        test_proof(&mmr, &nodes, 0..=1, &[], false);
        test_proof(&mmr, &nodes, 0..=1, &[], true);

        assert_eq!(
            mmr.prove_range(0..0, Some(mmr.len()), false).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove_range(0..0, Some(mmr.len()), true).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove_range(4..8, Some(mmr.len()), false).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove_range(4..8, Some(mmr.len()), true).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove_range(2..3, Some(mmr.len()), false).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            mmr.prove_range(2..3, Some(mmr.len()), true).map(|_| ()),
            Err(Error::ProveInvalidLeaves)
        );

        /*        root
         *       /    \
         *      2      \
         *    /   \     \
         *   0     1     3
         *   0     1     2
         */
        mmr.push(&nodes[2]).unwrap();

        test_proof(
            &mmr,
            &nodes,
            0..=0,
            &[Node::Store(1), Node::Store(3)],
            false,
        );
        test_proof(&mmr, &nodes, 0..=0, &[Node::Store(1), Node::Store(3)], true);
        test_proof(&mmr, &nodes, 2..=2, &[Node::Store(2)], false);
        test_proof(&mmr, &nodes, 2..=2, &[], true);

        /*
         *                      14
         *            /                    \
         *           6                      13
         *      /         \             /         \
         *     2           5           9           12           17
         *   /   \       /   \       /   \       /    \       /    \
         *  0     1     3     4     7     8     10    11     15    16    18
         *  0     1     2     3     4     5     6     7      8     9     10
         */
        for &v in nodes[3..].iter() {
            mmr.push(&v).unwrap();
        }

        let bagged_rhs = bagging(
            vec![
                Ok((mmr.store.get(18).unwrap(), 1)),
                Ok((mmr.store.get(17).unwrap(), 2)),
            ]
            .into_iter(),
        )
        .unwrap();

        test_proof(
            &mmr,
            &nodes,
            1..=1,
            &[
                Node::Store(0),
                Node::Store(5),
                Node::Store(13),
                Node::Hash(bagged_rhs.clone()),
            ],
            false,
        );
        test_proof(
            &mmr,
            &nodes,
            1..=1,
            &[
                Node::Store(5),
                Node::Store(13),
                Node::Hash(bagged_rhs.clone()),
            ],
            true,
        );

        test_proof(
            &mmr,
            &nodes,
            1..=6,
            &[
                Node::Store(0),
                Node::Store(11),
                Node::Hash(bagged_rhs.clone()),
            ],
            false,
        );
        test_proof(
            &mmr,
            &nodes,
            1..=6,
            &[Node::Store(11), Node::Hash(bagged_rhs)],
            true,
        );

        test_proof(
            &mmr,
            &nodes,
            7..=9,
            &[
                Node::Store(10),
                Node::Store(9),
                Node::Store(6),
                Node::Store(18),
            ],
            false,
        );
        test_proof(&mmr, &nodes, 7..=9, &[Node::Store(18)], true);

        test_proof(
            &mmr,
            &nodes,
            8..=9,
            &[Node::Store(14), Node::Store(18)],
            false,
        );
        test_proof(&mmr, &nodes, 8..=9, &[Node::Store(18)], true);

        test_proof(
            &mmr,
            &nodes,
            10..=10,
            &[Node::Store(14), Node::Store(17)],
            false,
        );
        test_proof(&mmr, &nodes, 10..=10, &[], true);
    }

    #[test]
    fn it_correctly_discards_invalid_range_proofs() {
        let nodes = vec![2, 3];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        for &v in nodes.iter() {
            mmr.push(&v).unwrap();
        }

        /*      2
         *    /   \
         *   0     1
         */
        let proof = mmr.prove_range(1..=1, Some(mmr.len()), false).unwrap();
        // Proof for wrong position.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(0, &2)]),
            Ok(false)
        );
        assert_eq!(
            proof.verify_with_start(&mmr.get_root().unwrap(), 0, &[2]),
            Ok(false)
        );
        // Proof for wrong value.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(1, &2)]),
            Ok(false)
        );
        assert_eq!(
            proof.verify_with_start(&mmr.get_root().unwrap(), 1, &[2]),
            Ok(false)
        );

        // Proof for less values.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(0, &2), (1, &3)]),
            Err(Error::InvalidProof)
        );
        assert_eq!(
            proof.verify_with_start(&mmr.get_root().unwrap(), 0, &[2, 3]),
            Err(Error::InvalidProof)
        );

        // Proof for non-leaves.
        assert_eq!(
            proof.verify(&mmr.get_root().unwrap(), &[(2, &0)]),
            Err(Error::ProveInvalidLeaves)
        );
        assert_eq!(
            proof.verify_with_start(&mmr.get_root().unwrap(), 2, &[0]),
            Err(Error::ProveInvalidLeaves)
        );
    }
}
