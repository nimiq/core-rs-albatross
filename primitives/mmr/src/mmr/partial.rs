use std::{collections::VecDeque, marker::PhantomData, ops::Range};

use crate::{
    error::Error,
    hash::{Hash, Merge},
    mmr::{
        peaks::PeakIterator,
        position::{leaf_number_to_index, Position},
        proof::RangeProof,
        utils::bagging,
        MerkleMountainRange,
    },
    store::{memory::MemoryTransaction, Store},
};

/// A struct that holds part of a MMR. Its main use is to construct a full MMR from a series of proofs.
pub struct PartialMerkleMountainRange<H, S: Store<H>> {
    store: S,
    num_proven_leaves: usize,
    size: Option<usize>,
    root: Option<H>,
    hash: PhantomData<H>,
}

impl<H: Merge + PartialEq + Clone, S: Store<H>> PartialMerkleMountainRange<H, S> {
    /// Creates a new partial MMR.
    pub fn new(store: S) -> Self {
        PartialMerkleMountainRange {
            store,
            num_proven_leaves: 0,
            size: None,
            root: None,
            hash: PhantomData,
        }
    }

    /// Returns the number of elements in the tree.
    pub fn len(&self) -> Option<usize> {
        self.size
    }

    /// Returns the number of already proven elements in the tree.
    pub fn proven_len(&self) -> usize {
        self.store.len()
    }

    /// Returns true if the tree is empty. It will return None if we don't know the size of the tree.
    pub fn is_empty(&self) -> Option<bool> {
        self.size.map(|size| size == 0)
    }

    /// Returns true if the tree is finished.
    pub fn is_finished(&self) -> bool {
        self.size
            .map(|size| size == self.store.len())
            .unwrap_or(false)
    }

    /// Transforms a partial MMR into a MMR, if the the partial MMR is finished.
    pub fn into_mmr(self) -> Result<MerkleMountainRange<H, S>, Self> {
        if self.is_finished() {
            return Ok(MerkleMountainRange::new(self.store));
        }
        Err(self)
    }

    /// Inserts a range proof.
    pub fn push_proof<T>(&mut self, range_proof: RangeProof<H>, leaves: &[T]) -> Result<H, Error>
    where
        T: Hash<H>,
    {
        // This is an optimized version of the proof's `compute_root` that is connected to a store.
        // Check if proof has right size.
        if self
            .len()
            .map(|len| len != range_proof.proof.mmr_size)
            .unwrap_or(false)
        {
            return Err(Error::InvalidProof);
        }

        let mut peaks_hashes = vec![];
        let mut store = MemoryTransaction::new(&mut self.store);
        let mut prev_i = 0; // Index into `leaves`.
        let mut proof_iter = range_proof.proof.nodes.iter();

        let num_proven_leaves = self.num_proven_leaves; // Required for the borrow checker.

        // The proof contains the nodes ordered by peak and then by level and then by index.
        // Start with the first peak that is at a position >= our first leaf index.
        for peak in PeakIterator::new(range_proof.proof.mmr_size) {
            // First, we collect all leaves belonging to this peak.
            let relevant_leaves: VecDeque<_> = leaves[prev_i..]
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    (
                        Position::from(leaf_number_to_index(num_proven_leaves + prev_i + i)),
                        item.hash(1),
                    )
                })
                .take_while(|(position, _)| position <= &peak)
                .collect();
            let new_node_range = leaf_number_to_index(self.num_proven_leaves + prev_i)
                ..leaf_number_to_index(self.num_proven_leaves + prev_i + relevant_leaves.len());
            prev_i += relevant_leaves.len();

            // If there is no leaf node, we need to take a proof node for this peak.
            // Then, we build up the tree similarly to the `calculate_root_for_peak`.
            if relevant_leaves.is_empty() {
                if let Some(hash) = Self::get_node(
                    peak.index,
                    &new_node_range,
                    &store,
                    &mut proof_iter,
                    range_proof.assume_previous,
                ) {
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
                    Self::push_proof_for_peak(
                        peak,
                        relevant_leaves,
                        new_node_range,
                        &mut store,
                        &mut proof_iter,
                        range_proof.assume_previous,
                    )?,
                    peak.num_leaves(),
                ));
            }
        }

        // Check if all leaves have been used and make sure there are no proof nodes left.
        if prev_i != leaves.len() || proof_iter.next().is_some() {
            return Err(Error::InvalidProof);
        }

        let root = bagging(peaks_hashes.into_iter().map(Ok).rev())?;

        // Compare root against stored root of previous proofs.
        if let Some(ref stored_root) = self.root {
            if root.ne(stored_root) {
                return Err(Error::InvalidProof);
            }
        }

        // All done, we can commit and update our state.
        store.commit();
        self.num_proven_leaves += leaves.len();
        self.size = Some(range_proof.proof.mmr_size);
        self.root = Some(root.clone());
        Ok(root)
    }

    /// Calculates the root.
    pub fn get_root(&self) -> Result<H, Error> {
        self.root.as_ref().cloned().ok_or(Error::EmptyTree)
    }

    /// Retrieves a node not part of the tree spanned by the newly added leaves.
    /// The tree spanned by the newly added leaves (leaf nodes `i..j`) consists of
    /// the range `leaf_number_to_index(i)..leaf_number_to_index(j)`.
    ///
    /// If `index` is above the `new_nodes_range`, we can only retrieve it from
    /// the proof's iterator `proof_iter`.
    /// If `index` is below the `new_nodes_range`, we retrieve it from the `store`
    /// and, in case `assume_previous` is `false`, we also check it against a node
    /// from `proof_iter`.
    /// If `index` is within the `new_nodes_range`, it is invalid to call this function
    /// and an error is returned.
    #[inline]
    fn get_node<'a, I>(
        index: usize,
        new_nodes_range: &Range<usize>,
        store: &MemoryTransaction<H, S>,
        proof_iter: &mut I,
        assume_previous: bool,
    ) -> Option<H>
    where
        H: 'a,
        I: Iterator<Item = &'a H>,
    {
        if new_nodes_range.contains(&index) {
            return None;
        }

        // If we are at a position left of our leftmost index, we take the hash
        // from the store (and potentially compare it with a proof node).
        if index < new_nodes_range.start {
            let hash = store.get(index)?;
            // Compare with proof node if given.
            if !assume_previous {
                let proof_hash = proof_iter.next()?;
                if hash.ne(proof_hash) {
                    return None;
                }
            }
            Some(hash)
        } else {
            proof_iter.next().cloned()
        }
    }

    /// Assumes a non-empty queue.
    fn push_proof_for_peak<'a, I>(
        peak_position: Position,
        mut queue: VecDeque<(Position, H)>,
        new_nodes_range: Range<usize>,
        store: &mut MemoryTransaction<H, S>,
        proof_iter: &mut I,
        assume_previous: bool,
    ) -> Result<H, Error>
    where
        H: 'a,
        I: Iterator<Item = &'a H>,
    {
        assert!(!queue.is_empty());

        // Shortcut if there's only one position, which coincides with the peak:
        // In that case, the leaf hash is actually the peak hash.
        if queue.len() == 1 && queue[0].0 == peak_position {
            if new_nodes_range.contains(&queue[0].0.index) {
                store.push(queue[0].1.clone());
            }
            return Ok(queue[0].1.clone());
        }

        // Since the order in which we build up the tree is different from the index order,
        // we temporarily store the hashes in a vector before moving them over into the store.
        let mut tmp_store = vec![None; new_nodes_range.end - new_nodes_range.start];

        // If the shortcut does not apply, we start with the positions in the list as a queue
        // of positions to look at.
        // We can think of this queue as a queue of elements that we can compute from
        // the data we have (including leaf nodes and proof nodes).
        // We iterate over this queue and move up the tree.
        while let Some((position, hash)) = queue.pop_front() {
            // Add positions into tmp store.
            if new_nodes_range.contains(&position.index) {
                tmp_store[position.index - new_nodes_range.start] = Some(hash.clone());
            }

            // Stop once we reached the peak.
            if position == peak_position {
                // But first transfer our `tmp_store` into the real store.
                let tmp_store: Option<Vec<_>> = tmp_store.into_iter().collect();
                let tmp_store = tmp_store.ok_or(Error::InvalidProof)?;
                store.append(tmp_store);
                return Ok(hash);
            }

            // Otherwise, check if we need to take a proof node on this level.
            // Following the same process as for constructing the proof, we can check whether
            // the node required is at the front of the queue. If not, we need to take a proof
            // node instead.
            let sibling = position.sibling();
            let sibling_hash = if queue.front().map(|(pos, _)| pos) == Some(&sibling) {
                // If the sibling is part of the queue, take its hash.
                let hash = queue.pop_front().unwrap().1;
                if new_nodes_range.contains(&sibling.index) {
                    tmp_store[sibling.index - new_nodes_range.start] = Some(hash.clone());
                }
                hash
            } else {
                Self::get_node(
                    sibling.index,
                    &new_nodes_range,
                    store,
                    proof_iter,
                    assume_previous,
                )
                .ok_or(Error::InvalidProof)?
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

#[cfg(test)]
mod tests {
    use std::cmp;

    use nimiq_test_log::test;

    use super::*;
    use crate::{
        error::Error,
        mmr::{utils::test_utils::TestHash, MerkleMountainRange},
        store::{memory::MemoryStore, Store},
    };

    #[test]
    fn it_correctly_verifies_range_proofs() {
        /// Proves a whole tree by chunking `leaves` into chunks of size `chunk_size`
        /// and applying the range proofs to a partial MMR.
        fn test_proof<S: Store<TestHash>>(
            mmr: &MerkleMountainRange<TestHash, S>,
            leaves: &[usize],
            chunk_size: usize,
            assume_previous: bool,
        ) {
            let mut tmp_mmr = MerkleMountainRange::new(MemoryStore::new());
            let mut pmmr = PartialMerkleMountainRange::new(MemoryStore::new());

            for i in (0..leaves.len()).step_by(chunk_size) {
                let to_prove = i..cmp::min(i + chunk_size, leaves.len());
                let proof = mmr
                    .prove_range(to_prove.clone(), Some(mmr.len()), assume_previous)
                    .unwrap();

                assert!(
                    !pmmr.is_finished(),
                    "Failed for {} leaves with chunk_size {} in chunk {}",
                    leaves.len(),
                    chunk_size,
                    i
                );
                assert_eq!(
                    pmmr.push_proof(proof, &leaves[to_prove.clone()]),
                    mmr.get_root(),
                    "Failed for {} leaves with chunk_size {} in chunk {}",
                    leaves.len(),
                    chunk_size,
                    i
                );

                // Check if partial length is correct.
                for &v in &leaves[to_prove] {
                    tmp_mmr.push(&v).unwrap();
                }
                assert_eq!(
                    pmmr.proven_len(),
                    tmp_mmr.len(),
                    "Failed for {} leaves with chunk_size {} in chunk {}",
                    leaves.len(),
                    chunk_size,
                    i
                );
            }

            assert!(
                pmmr.is_finished(),
                "Failed for {} leaves with chunk_size {}",
                leaves.len(),
                chunk_size,
            );
            let mmr2 = if let Ok(mmr2) = pmmr.into_mmr() {
                mmr2
            } else {
                panic!("Should be finished!")
            };
            assert_eq!(mmr2.len(), mmr.len());
            assert_eq!(mmr2.get_root(), mmr.get_root());
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
        for chunk_size in 1..=2 {
            test_proof(&mmr, &nodes[..2], chunk_size, false);
            test_proof(&mmr, &nodes[..2], chunk_size, true);
        }

        /*        root
         *       /    \
         *      2      \
         *    /   \     \
         *   0     1     3
         *   0     1     2
         */
        mmr.push(&nodes[2]).unwrap();

        for chunk_size in 1..=3 {
            test_proof(&mmr, &nodes[..3], chunk_size, false);
            test_proof(&mmr, &nodes[..3], chunk_size, true);
        }

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

        for chunk_size in 1..=10 {
            test_proof(&mmr, &nodes, chunk_size, false);
            test_proof(&mmr, &nodes, chunk_size, true);
        }
    }

    #[test]
    fn it_correctly_discards_invalid_range_proofs() {
        let nodes = [2, 3];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        for &v in nodes.iter() {
            mmr.push(&v).unwrap();
        }

        /*      2
         *    /   \
         *   0     1
         */
        let mut pmmr = PartialMerkleMountainRange::new(MemoryStore::new());

        // Proof for wrong position.
        let proof = mmr.prove_range(1..=1, Some(mmr.len()), false).unwrap();
        assert_ne!(pmmr.push_proof(proof, &[2]), mmr.get_root());

        let proof = mmr.prove_range(1..=1, Some(mmr.len()), true).unwrap();
        assert_ne!(pmmr.push_proof(proof, &[2]), mmr.get_root());

        // Proof for wrong value.
        let proof = mmr.prove_range(0..=0, Some(mmr.len()), false).unwrap();
        assert_ne!(pmmr.push_proof(proof, &[3]), mmr.get_root());

        let proof = mmr.prove_range(0..=0, Some(mmr.len()), true).unwrap();
        assert_ne!(pmmr.push_proof(proof, &[3]), mmr.get_root());

        // Proof for less values.
        let proof = mmr.prove_range(0..=1, Some(mmr.len()), false).unwrap();
        assert_eq!(pmmr.push_proof(proof, &[2]), Err(Error::InvalidProof));

        let proof = mmr.prove_range(0..=1, Some(mmr.len()), true).unwrap();
        assert_eq!(pmmr.push_proof(proof, &[2]), Err(Error::InvalidProof));

        // Proof for non-leaves.
        let proof = mmr.prove_range(1..=1, Some(mmr.len()), false).unwrap();
        assert_eq!(pmmr.push_proof(proof, &[2, 3, 5]), Err(Error::InvalidProof));

        let proof = mmr.prove_range(1..=1, Some(mmr.len()), true).unwrap();
        assert_eq!(pmmr.push_proof(proof, &[2, 3, 5]), Err(Error::InvalidProof));
    }
}
