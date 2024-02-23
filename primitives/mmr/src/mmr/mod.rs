use std::{
    collections::{HashSet, VecDeque},
    marker::PhantomData,
    ops::RangeBounds,
};

use self::{proof::SizeProof, utils::prove_num_leaves};
use crate::{
    error::Error,
    hash::{Hash, Merge},
    mmr::{
        peaks::{PeakIterator, RevPeakIterator},
        position::{index_to_height, leaf_number_to_index, Position},
        proof::{Proof, RangeProof},
        utils::bagging,
    },
    store::{memory::MemoryTransaction, LightStore, Store},
};

pub mod partial;
pub mod peaks;
pub mod position;
pub mod proof;
mod utils;

/// This is the main struct for the Merkle Mountain Range. It simply consists of a store for the
/// nodes and the number of leaves.
pub struct MerkleMountainRange<H, S: Store<H>> {
    store: S,
    num_leaves: usize,
    hash: PhantomData<H>,
}

impl<H: Merge + Clone, S: Store<H>> MerkleMountainRange<H, S> {
    /// Creates a new Merkle Mountain Range from a given Store.
    pub fn new(store: S) -> Self {
        let mut mmr = MerkleMountainRange {
            store,
            num_leaves: 0,
            hash: PhantomData,
        };

        // The number of leaves is the sum over 2^height for all full binary trees.
        if !mmr.is_empty() {
            mmr.num_leaves = mmr.peaks().map(|peak_pos| peak_pos.num_leaves()).sum();
        };

        mmr
    }

    /// Returns a element of the tree by its index.
    pub fn get(&self, index: usize) -> Option<H> {
        self.store.get(index)
    }

    /// Returns the number of elements in the tree.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Returns a leaf of the tree by its leaf index.
    pub fn get_leaf(&self, leaf_index: usize) -> Option<H> {
        // Note that leaf index starts at 0.
        if leaf_index >= self.num_leaves() {
            return None;
        }

        self.store.get(leaf_number_to_index(leaf_index))
    }

    /// Returns the number of leaf hashes in the tree.
    pub fn num_leaves(&self) -> usize {
        self.num_leaves
    }

    /// Returns true if the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.store.len() == 0
    }

    /// Inserts an element and returns the corresponding leaf index.
    /// The leaf index is not the internal index within the tree,
    /// but a leaf index of i means that it is the i-th leaf (starting to count at 0).
    pub fn push<T>(&mut self, elem: &T) -> Result<usize, Error>
    where
        T: Hash<H>,
    {
        // Set new leaf index.
        let num_leaves = self.num_leaves();

        let index = self.len();
        let mut pos = Position::from(index);

        let mut store = MemoryTransaction::new(&mut self.store);
        store.push(elem.hash(pos.num_leaves() as u64));

        // Hash up as long as possible (as long as we're the right child of the parent).
        while pos.right_node {
            let left_pos = pos.sibling();

            let left_elem = store.get(left_pos.index).ok_or(Error::InconsistentStore)?;
            let right_elem = store.get(pos.index).ok_or(Error::InconsistentStore)?;

            pos = pos.parent();

            // Prefix the merged hash with the number of leaf elements below that hash, which are
            // 2^height as it is a perfect binary tree.
            let parent_elem = left_elem.merge(&right_elem, pos.num_leaves() as u64);
            store.push(parent_elem);
        }

        store.commit();

        // Update num_leaves.
        self.num_leaves += 1;
        let leaf_index = num_leaves;
        Ok(leaf_index)
    }

    /// Removes the most recent leaf.
    pub fn remove_back(&mut self) -> Result<(), Error> {
        if self.is_empty() {
            return Err(Error::EmptyTree);
        }

        // If the latest entry is at height h, there are h intermediate nodes
        // to be removed until we reach the most recent leaf (which we also want to remove).
        let height = index_to_height(self.len() - 1);
        self.store.remove_back(height + 1);
        Ok(())
    }

    /// Calculates the root.
    pub fn get_root(&self) -> Result<H, Error> {
        if self.is_empty() {
            return Ok(H::empty(0));
        }

        // The peak hashes are not given in reverse order as we pop them from the end.
        let it = self.rev_peaks().map(|peak_pos| {
            Ok((
                self.store
                    .get(peak_pos.index)
                    .ok_or(Error::InconsistentStore)?,
                peak_pos.num_leaves(),
            ))
        });

        bagging(it)
    }

    /// Generates the size proof.
    /// If the verifier has a MMR that is a subtree of the prover's MMR (meaning that the prover's
    /// MMR is simply the verifier's MMR with more leaves added), you can set `verifier_state` to
    /// the length of the verifier's MMR. This will construct a proof that is valid to the verifier.
    pub fn prove_num_leaves<T: Hash<H>, F: Fn(H) -> Option<T>>(
        &self,
        f: F,
        verifier_state: Option<usize>,
    ) -> Result<SizeProof<H, T>, Error> {
        // Return the history root.
        if self.is_empty() {
            return Ok(SizeProof::EmptyTree);
        }

        // Determine length.
        let length = match verifier_state {
            None => self.len(),
            Some(x) => {
                if x > self.len() {
                    return Err(Error::ProveInvalidLeaves);
                } else {
                    x
                }
            }
        };

        let it = RevPeakIterator::new(length).map(|peak_pos| {
            Ok((
                self.store
                    .get(peak_pos.index)
                    .ok_or(Error::InconsistentStore)?,
                peak_pos.num_leaves(),
                peak_pos
                    .left_child()
                    .and_then(|pos| self.store.get(pos.index)),
                peak_pos
                    .right_child()
                    .and_then(|pos| self.store.get(pos.index)),
            ))
        });

        prove_num_leaves(it, f)
    }

    /// Returns a Merkle proof for the given positions (representing the leaf indexes).
    /// If the verifier has a MMR that is a subtree of the prover's MMR (meaning that the prover's
    /// MMR is simply the verifier's MMR with more leaves added), you can set `verifier_state` to
    /// the length of the verifier's MMR. This will construct a proof that is valid to the verifier.
    pub fn prove(
        &self,
        positions: &[usize],
        verifier_state: Option<usize>,
    ) -> Result<Proof<H>, Error> {
        // Determine length.
        let length = match verifier_state {
            None => self.len(),
            Some(x) => {
                if x > self.len() {
                    return Err(Error::ProveInvalidLeaves);
                } else {
                    x
                }
            }
        };

        // Sort positions and check that we only prove leaves.
        let positions: Result<Vec<Position>, Error> = positions
            .iter()
            .map(|&index| {
                let pos = Position::from(leaf_number_to_index(index));
                // Make sure that we only ever prove leaf nodes.
                if pos.index >= length {
                    return Err(Error::ProveInvalidLeaves);
                }
                Ok(pos)
            })
            .collect();

        let mut positions = positions?;

        positions.sort_unstable();

        self.prove_positions(positions, length, false)
    }

    /// Returns a Merkle proof for a range of leaf indexes.
    /// If the verifier has a MMR that is a subtree of the prover's MMR (meaning that the prover's
    /// MMR is simply the verifier's MMR with more leaves added), you can set `verifier_state` to
    /// the length of the verifier's MMR. This will construct a proof that is valid to the verifier.
    /// If the verifier is receiving several consecutive range proofs, you can set `assume_previous`
    /// to true. This will reduce the size of the proofs.
    pub fn prove_range<R: RangeBounds<usize> + Iterator<Item = usize>>(
        &self,
        range: R,
        verifier_state: Option<usize>,
        assume_previous: bool,
    ) -> Result<RangeProof<H>, Error> {
        // Determine length.
        let length = match verifier_state {
            None => self.len(),
            Some(x) => {
                if x > self.len() {
                    return Err(Error::ProveInvalidLeaves);
                } else {
                    x
                }
            }
        };

        // Find all leaves in the range.
        // If we know whether the first leaf is on the left-hand side or right-hand side,
        // we can simply alternate this for the following positions.
        let mut it = range
            .into_iter()
            .map(leaf_number_to_index) // Map to tree internal indices.
            .filter(|i| *i < self.len())
            .peekable();

        // Find out which side our first leaf is on.
        let first_position = Position::from(*it.peek().ok_or(Error::ProveInvalidLeaves)?);

        // Then use this and produce positions without extra calls to `index_to_height`.
        let leaves: Vec<_> = it
            .scan(!first_position.right_node, |st, index| {
                *st = !*st; // Alternate bit.
                Some(Position {
                    index,
                    height: 0,
                    right_node: *st,
                })
            })
            .collect();

        // If we assume the previous proof to be known, we can remove all proof nodes with an index <= our first leaf.
        Ok(RangeProof {
            proof: self.prove_positions(leaves, length, assume_previous)?,
            assume_previous,
        })
    }

    /// Private function that returns a Merkle proof for the given positions. Assumes `positions`
    /// to be sorted already.
    fn prove_positions(
        &self,
        positions: Vec<Position>,
        tree_length: usize,
        assume_previous: bool,
    ) -> Result<Proof<H>, Error> {
        // The slice of positions cannot be empty. Except if the tree is also empty.
        if positions.is_empty() {
            return if tree_length == 0 {
                // In this case we return an empty proof.
                Ok(Proof::new(0))
            } else {
                Err(Error::ProveInvalidLeaves)
            };
        }

        let mut proof = Proof::new(tree_length);
        // Shortcut for trees of size 1.
        if proof.mmr_size == 1 && positions.len() == 1 && positions[0].index == 0 {
            return Ok(proof);
        }

        let lower_bound = if assume_previous {
            positions[0].index
        } else {
            0
        };

        // Add proof nodes for each peak individually.
        let mut prev_i = 0; // Index into the `positions` Vec.
        let mut baggable_peaks = vec![]; // Collect empty peak positions since the last non-empty one (for bagging).
        for peak in PeakIterator::new(tree_length) {
            // Find all positions belonging to this peak.
            let peak_positions = &positions[prev_i..];
            let i_offset = peak_positions
                .iter()
                .position(|&pos| pos > peak)
                .unwrap_or(peak_positions.len());

            if i_offset == 0 {
                // Another peak with no positions to prove.
                baggable_peaks.push(peak);
            } else {
                // A peak with positions to prove, reset.
                baggable_peaks.clear();
            }

            self.prove_for_peak(
                &mut proof.nodes,
                &peak_positions[..i_offset],
                peak,
                lower_bound,
            )?;
            prev_i += i_offset;
        }

        // Make sure there were no positions larger than our size.
        if prev_i < positions.len() {
            return Err(Error::ProveInvalidLeaves);
        }

        // For now, our proof will contain individual peaks if there are no positions to prove under.
        // However, we can improve on that on the right hand side by bagging all those.
        // We remove those from our proof and replace them by a single bagged node.
        if !baggable_peaks.is_empty() {
            let baggable_peak_hashes = proof
                .nodes
                .split_off(proof.nodes.len() - baggable_peaks.len());

            proof.nodes.push(bagging(
                baggable_peak_hashes
                    .into_iter()
                    .zip(baggable_peaks.into_iter().map(|peak| peak.num_leaves()))
                    .map(Ok)
                    .rev(),
            )?);
        }

        Ok(proof)
    }

    /// Private function that creates a Merkle proof for a single peak.
    /// The `lower_bound` parameter is used to exclude proof nodes below a certain index.
    /// If no proof nodes should be excluded, the parameter is set to 0.
    fn prove_for_peak(
        &self,
        proof_nodes: &mut Vec<H>,
        positions: &[Position],
        peak_position: Position,
        lower_bound: usize,
    ) -> Result<(), Error> {
        // Shortcut: If there is no position to prove below this peak,
        // we only need to add the peak itself.
        if positions.is_empty() {
            if peak_position.index >= lower_bound {
                proof_nodes.push(
                    self.store
                        .get(peak_position.index)
                        .ok_or(Error::InconsistentStore)?,
                );
            }
            return Ok(());
        }

        // Shortcut if there's only one position, which coincides with the peak:
        // In that case, there are no proof nodes to add.
        if positions.len() == 1 && positions[0] == peak_position {
            return Ok(());
        }

        // If no shortcut applies, we start with the positions in the list as a queue
        // of positions to look at.
        // We can think of this queue as a queue of elements that we can compute from
        // the data we have (including leaf nodes and proof nodes).
        // We iterate over this queue, add non-computable siblings to our proof nodes,
        // and move up the tree.
        let mut queue: VecDeque<_> = positions.iter().copied().collect();
        while let Some(position) = queue.pop_front() {
            // Stop once we reached the peak.
            if position == peak_position {
                break;
            }

            // Otherwise, check if we need to add a proof node on this level.
            // We need a proof node whenever only one of two siblings is part of the computable
            // nodes.
            // We determine this as follows:
            // - If our current position is the left sibling, we can check whether the right sibling
            //   is computable by checking whether it is next in queue.
            //   If that is true, we will remove the right sibling from the queue and process both
            //   the current position and the right sibling at the same time by not adding any proof
            //   nodes.
            //   If that is false, we need to add the sibling as a proof node.
            // - If our current position is the right sibling, we know by the previous operation
            //   that the left sibling was not computable (otherwise the right sibling wouldn't
            //   be part of the list anymore). Thus, we need to add the sibling as a proof node.
            // The process above can be simplified to the following code:
            let sibling = position.sibling();
            if queue.front() == Some(&sibling) {
                // If the sibling is also to be proven, we do not need to process it anymore
                // (as both siblings were in the queue, we do not need to add any proof nodes).
                queue.pop_front();
            } else {
                // If the sibling is not to be proven, we need to add it as a proof node.
                if sibling.index >= lower_bound {
                    proof_nodes.push(
                        self.store
                            .get(sibling.index)
                            .ok_or(Error::InconsistentStore)?,
                    );
                }
            }

            // Add the parent to the queue.
            queue.push_back(position.parent());
        }
        Ok(())
    }

    /// Returns an iterator for the MMR peaks in normal order. From biggest/left to smallest/right.
    fn peaks(&self) -> PeakIterator {
        PeakIterator::new(self.len())
    }

    /// Returns an iterator for the MMR peaks in reverse order. From smallest/right to biggest/left.
    fn rev_peaks(&self) -> RevPeakIterator {
        RevPeakIterator::new(self.len())
    }
}

impl<H: Merge + Clone + PartialEq, S: Store<H>> MerkleMountainRange<H, S> {
    /// Tries to find a hash in the tree in O(n) and returns its leaf index.
    pub fn find<T>(&self, elem: T) -> Option<usize>
    where
        T: Hash<H>,
    {
        let h = elem.hash(1);

        (0..self.num_leaves()).find(|&i| {
            if let Some(leaf) = self.get_leaf(i) {
                h.eq(&leaf)
            } else {
                false
            }
        })
    }
}

/// This is the main struct for the Peaks Merkle Mountain Range.
/// It is a MMR where only peaks are stored.
pub struct PeaksMerkleMountainRange<H, S: LightStore<H>> {
    store: S,
    len: usize,
    num_leaves: usize,
    hash: PhantomData<H>,
}

impl<H: Merge + Clone, S: LightStore<H>> PeaksMerkleMountainRange<H, S> {
    /// Creates a new Peaks Merkle Mountain Range
    pub fn new(store: S) -> Self {
        let initial_len = store.len();

        let mut mmr = PeaksMerkleMountainRange {
            store,
            num_leaves: 0,
            hash: PhantomData,
            len: initial_len,
        };

        // The number of leaves is the sum over 2^height for all full binary trees.
        if !mmr.is_empty() {
            mmr.num_leaves = mmr.peaks().map(|peak_pos| peak_pos.num_leaves()).sum();
        };

        mmr
    }

    /// Returns the number of elements in the tree.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns the number of leaf hashes in the tree.
    pub fn num_leaves(&self) -> usize {
        self.num_leaves
    }

    /// Returns true if the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Inserts an element and returns the corresponding leaf index.
    /// The leaf index is not the internal index within the tree,
    /// but a leaf index of i means that it is the i-th leaf (starting to count at 0).
    pub fn push<T>(&mut self, elem: &T) -> Result<usize, Error>
    where
        T: Hash<H>,
    {
        // We will use this hash set to collect items that might be deleted after the end of this operation
        let mut temp_elements = HashSet::new();

        // Set new leaf index.
        let num_leaves = self.num_leaves();

        let index = self.len();
        let mut pos = Position::from(index);

        let old_peaks: HashSet<usize> = self.peaks().map(|pos| pos.index).collect();

        self.store
            .insert(elem.hash(pos.num_leaves() as u64), pos.index);

        temp_elements.insert(pos.index);

        self.len += 1;

        // Hash up as long as possible (as long as we're the right child of the parent).
        while pos.right_node {
            let left_pos = pos.sibling();

            let left_elem = self
                .store
                .get(left_pos.index)
                .ok_or(Error::InconsistentStore)?;
            let right_elem = self.store.get(pos.index).ok_or(Error::InconsistentStore)?;

            pos = pos.parent();

            // Prefix the merged hash with the number of leaf elements below that hash, which are
            // 2^height as it is a perfect binary tree.
            let parent_elem = left_elem.merge(&right_elem, pos.num_leaves() as u64);
            self.store.insert(parent_elem, pos.index);
            temp_elements.insert(pos.index);
            self.len += 1;
        }

        let new_peaks: HashSet<usize> = self.peaks().map(|pos| pos.index).collect();

        // Values that are in old that are not in the new positions
        let to_remove: HashSet<usize> = old_peaks.difference(&new_peaks).cloned().collect();

        // Finally update the store
        for pos in to_remove {
            self.store.remove(pos);
        }

        // Remove any temporal element that was created that is no longer needed
        for pos in temp_elements {
            if !new_peaks.contains(&pos) {
                self.store.remove(pos);
            }
        }

        // Update num_leaves.
        self.num_leaves += 1;
        Ok(num_leaves)
    }

    /// Calculates the root.
    pub fn get_root(&self) -> Result<H, Error> {
        if self.is_empty() {
            return Ok(H::empty(0));
        }

        // The peak hashes are not given in reverse order as we pop them from the end.
        let it = self.rev_peaks().map(|peak_pos| {
            Ok((
                self.store
                    .get(peak_pos.index)
                    .ok_or(Error::InconsistentStore)?,
                peak_pos.num_leaves(),
            ))
        });

        bagging(it)
    }

    /// Returns an iterator for the MMR peaks in normal order. From biggest/left to smallest/right.
    fn peaks(&self) -> PeakIterator {
        PeakIterator::new(self.len())
    }

    /// Returns an iterator for the MMR peaks in reverse order. From smallest/right to biggest/left.
    fn rev_peaks(&self) -> RevPeakIterator {
        RevPeakIterator::new(self.len())
    }
}

#[cfg(test)]
mod tests {
    use std::vec;

    use nimiq_test_log::test;

    use super::*;
    use crate::{
        mmr::utils::test_utils::{hash_mmr, TestHash},
        store::memory::{LightMemoryStore, MemoryStore},
    };

    #[test]
    fn it_correctly_constructs_trees() {
        let nodes = vec![2, 3, 5, 7, 11, 13, 17, 19, 23, 29];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        for (i, v) in nodes.clone().into_iter().enumerate() {
            // Add value.
            let index = mmr.push(&v);
            assert_eq!(index, Ok(i));
            // Check properties.
            assert_eq!(mmr.num_leaves(), i + 1);
            assert_eq!(mmr.find(v), Some(i));
            // Check hash.
            assert_eq!(mmr.get_root(), Ok(hash_mmr(&nodes[..i + 1])))
        }
    }

    #[test]
    fn it_correctly_constructs_ptrees() {
        let nodes = vec![2, 3, 5, 7, 11, 13, 17, 19, 23, 29];

        let store = LightMemoryStore::new();
        let mut mmr = PeaksMerkleMountainRange::<TestHash, _>::new(store);

        for (i, v) in nodes.clone().into_iter().enumerate() {
            // Add value.
            let index = mmr.push(&v);
            assert_eq!(index, Ok(i));
            // Check properties.
            assert_eq!(mmr.num_leaves(), i + 1);
            // Check hash.
            assert_eq!(mmr.get_root(), Ok(hash_mmr(&nodes[..i + 1])))
        }
    }

    #[test]
    fn it_correctly_proves_num_leaves() {
        let nodes = vec![2, 3, 5, 7, 11, 13, 17, 19, 23, 29];
        let mut hashes = vec![];

        let store = MemoryStore::new();
        let mut mmr = MerkleMountainRange::<TestHash, _>::new(store);

        // Check empty size proof.
        let size_proof = mmr
            .prove_num_leaves(
                |hash| hashes.iter().position(|h| h == &hash).map(|pos| nodes[pos]),
                None,
            )
            .unwrap();

        assert!(size_proof.verify(&mmr.get_root().unwrap()));
        assert_eq!(size_proof.size(), 0);

        // Check size proof for multiple sizes.
        let mut root_hashes = vec![];
        for (i, v) in nodes.clone().into_iter().enumerate() {
            // Add value.
            let index = mmr.push(&v).unwrap();
            let hash = mmr.get_leaf(index).unwrap();
            hashes.push(hash);

            let size_proof = mmr
                .prove_num_leaves(
                    |hash| hashes.iter().position(|h| h == &hash).map(|pos| nodes[pos]),
                    None,
                )
                .unwrap();

            let root_hash = mmr.get_root().unwrap();
            assert!(size_proof.verify(&root_hash));
            assert_eq!(size_proof.size() as usize, i + 1);

            root_hashes.push(root_hash);
        }

        // Check size proof for different verifier states.
        for i in 0..mmr.num_leaves {
            let size_proof = mmr
                .prove_num_leaves(
                    |hash| hashes.iter().position(|h| h == &hash).map(|pos| nodes[pos]),
                    Some(leaf_number_to_index(i + 1)),
                )
                .unwrap();

            assert!(size_proof.verify(&root_hashes[i]));
            assert_eq!(size_proof.size() as usize, i + 1);
        }
    }
}
