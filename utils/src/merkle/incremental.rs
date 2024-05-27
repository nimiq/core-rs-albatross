use std::mem;

use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, HashOutput, Hasher, SerializeContent};
use nimiq_serde::{Deserialize, Serialize};

/// An IncrementalMerkleProofBuilder can construct sequentially verifiable merkle proofs for a large list of data.
/// The main difference to the PartialMerkleProofBuilder is that a different tree generation algorithm is used
/// that is optimized to incrementally add items to the tree.
///
/// Note that the trees are incompatible and return different root hashes.
///
/// The data can then be split into chunks and each chunk has its own (small) proof.
/// Each proof can be verified by taking the previous proof's result into account (to minimise the amount of work required).
/// This way, the large list of data can be transmitted in smaller chunks and one can incrementally verify
/// the correctness of these chunks.
pub struct IncrementalMerkleProofBuilder<H: HashOutput> {
    tree: Vec<Vec<H>>,
    chunk_size: usize,
}

impl<H: HashOutput> IncrementalMerkleProofBuilder<H> {
    pub fn new(chunk_size: usize) -> Result<Self, IncrementalMerkleProofError> {
        if chunk_size == 0 {
            return Err(IncrementalMerkleProofError::InvalidChunkSize);
        }

        Ok(IncrementalMerkleProofBuilder {
            tree: vec![vec![]],
            chunk_size,
        })
    }

    pub fn len(&self) -> usize {
        self.tree[0].len()
    }

    pub fn is_empty(&self) -> bool {
        self.tree[0].is_empty()
    }

    /// Returns the chunk index the item is in.
    pub fn push_item<T: SerializeContent>(&mut self, value: &T) -> usize {
        self.push(H::Builder::default().chain(value).finish())
    }

    /// Returns the chunk index the hash is in.
    pub fn push(&mut self, hash: H) -> usize {
        // Add to the leaves.
        self.tree[0].push(hash);
        self.update();

        self.tree[0].len() / self.chunk_size
    }

    /// Removes the last element from the tree.
    pub fn pop(&mut self) -> Option<H> {
        let result = self.tree[0].pop()?;
        self.update();

        Some(result)
    }

    /// Updates the merkle tree assuming that an element has been added or removed.
    fn update(&mut self) {
        if self.tree[0].is_empty() {
            return;
        }

        // Iteratively build up the Merkle tree.
        let mut current_level = 0;
        let mut current_pos = self.tree[0].len() - 1;

        // If current_pos is 0, this is the only element on the level and there's nothing to hash.
        while current_pos > 0 {
            // If there are entries not required anymore, drop them.
            if current_pos + 1 != self.tree[current_level].len() {
                self.tree[current_level].pop();
            }

            // Borrow on level for convenience.
            let level = &self.tree[current_level];

            // If the current position is uneven, we can hash two elements together.
            let hash = if current_pos % 2 == 1 {
                H::Builder::default()
                    .chain(&level[current_pos - 1])
                    .chain(&level[current_pos])
                    .finish()
            } else {
                // Otherwise, we only have one element.
                level[current_pos].clone()
            };

            // Push hash to next level.
            current_level += 1;
            current_pos /= 2;
            if current_level >= self.tree.len() {
                self.tree.push(vec![]);
            }
            // If the position already exists, modify, else, push.
            if current_pos >= self.tree[current_level].len() {
                self.tree[current_level].push(hash);
            } else {
                self.tree[current_level][current_pos] = hash;
            }
        }

        // Remove top level if not needed anymore.
        if current_level + 1 != self.tree.len() {
            self.tree.pop();
        }
    }

    pub fn root(&self) -> Option<&H> {
        self.tree.last()?.first()
    }

    pub fn chunks(&self) -> Vec<IncrementalMerkleProof<H>> {
        let num_chunks = self.len().div_ceil(self.chunk_size);
        let mut chunks = Vec::with_capacity(num_chunks);
        for i in 0..num_chunks {
            chunks.push(self.chunk(i).unwrap());
        }
        chunks
    }

    pub fn chunk(&self, index: usize) -> Option<IncrementalMerkleProof<H>> {
        if index * self.chunk_size >= self.len() {
            return None;
        }

        let mut proof = IncrementalMerkleProof::empty(self.len());

        // The proof consists of all nodes on the right hand side of the leaves hashing upwards.
        // We put our current index at the rightmost node in our chunk.
        let mut current_pos = (index + 1) * self.chunk_size - 1;
        let mut current_level = 0;
        // If the node is the last on that level,
        // we don't need to add any more nodes as we reached the end of the tree.
        while current_pos + 1 < self.tree[current_level].len() {
            // The next node on the level is at current_index + 1.
            // We only need to add it to the proof, if our current index is
            // at an even position (left side of two hashes).
            if current_pos % 2 == 0 {
                proof
                    .nodes
                    .push(self.tree[current_level][current_pos + 1].clone());
            }

            // Progress one level up.
            current_level += 1;
            current_pos /= 2;
            if current_level >= self.tree.len() {
                break;
            }
        }

        Some(proof)
    }
}

/// An IncrementalMerkleProof is a merkle proof for a single chunk from a large set of data.
/// These proofs can only be verified incrementally, i.e., one has to start with the first chunk of data.
/// The proof for the second chunk then takes as an input the result of the first chunk's proof and so on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "H: HashOutput"))]
pub struct IncrementalMerkleProof<H: HashOutput> {
    total_len: u32,
    nodes: Vec<H>,
}

#[derive(Debug)]
pub struct IncrementalMerkleProofResult<H: HashOutput> {
    /// The calculated root of the merkle proof.
    root: H,
    /// A set of hashes in the merkle tree that are used in the next proof's verification.
    helper_nodes: Vec<H>,
    /// The next index after this proof's leaf nodes in the overall data.
    next_index: usize,
}

impl<H: HashOutput> IncrementalMerkleProofResult<H> {
    #[inline]
    pub fn root(&self) -> &H {
        &self.root
    }

    #[inline]
    pub fn helper_nodes(&self) -> &[H] {
        &self.helper_nodes
    }

    #[inline]
    pub fn next_index(&self) -> usize {
        self.next_index
    }
}

impl<H: HashOutput> IncrementalMerkleProof<H> {
    pub fn empty(total_len: usize) -> Self {
        IncrementalMerkleProof {
            total_len: total_len as u32,
            nodes: Vec::new(),
        }
    }

    pub fn compute_root_from_values<T: SerializeContent>(
        &self,
        chunk_values: &[T],
        previous_result: Option<&IncrementalMerkleProofResult<H>>,
    ) -> Result<IncrementalMerkleProofResult<H>, IncrementalMerkleProofError> {
        let hashes: Vec<H> = chunk_values
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        self.compute_root(&hashes, previous_result)
    }

    pub fn compute_root(
        &self,
        chunk_hashes: &[H],
        previous_result: Option<&IncrementalMerkleProofResult<H>>,
    ) -> Result<IncrementalMerkleProofResult<H>, IncrementalMerkleProofError> {
        let index_offset = previous_result.map(|r| r.next_index).unwrap_or(0);
        let empty_vec = Vec::new();
        let helper_nodes = previous_result
            .map(|r| &r.helper_nodes)
            .unwrap_or(&empty_vec);

        // The index of the helper node we use next.
        let mut helper_index = 0;
        // The index of the proof node we use next.
        let mut proof_index = 0;

        // The helper nodes for the next proof.
        let mut helper_output = Vec::new();

        // The hashes on the current level.
        let mut current_level = chunk_hashes;
        let mut current_level_owned = vec![];
        // Store whether a proof node has been used.
        let mut current_proof_nodes = BitSet::new();
        // The leftmost index on the current level for this chunk.
        let mut level_leftmost = index_offset;
        // The rightmost index on the current level for this chunk.
        let mut level_rightmost = index_offset + chunk_hashes.len() - 1;
        // What index are we at right now on the current level.
        // Start with the left child even if it is not part of our chunk.
        let mut current_position = (level_leftmost / 2) * 2;
        // The length of the current level.
        let mut level_len = self.total_len;
        // The resulting hashes on the next level.
        let mut next_level = vec![];
        let mut next_proof_nodes = BitSet::new();
        // Used to map the first position to 0 in the bitsets.
        let mut level_offset = current_position;

        // Continue until we only have the root left.
        let depth = f64::from(self.total_len).log2().ceil() as u32;
        for _ in 0..depth {
            // Process as much as possible on the current level.
            while current_position <= level_rightmost {
                // We always try to hash two hashes together, except if we reached the end of the
                // level and there are no more nodes to process.
                // There are two special cases:
                // 1. The first item of the chunk is the right child of the upper layer.
                //    Then the left child must be part of the helper nodes.
                // 2. The last item of the chunk is the left child of the upper layer and
                //    we haven't reached the end yet. Then, we need to take a proof node.
                let left_hash = if current_position < level_leftmost {
                    helper_index += 1;
                    helper_nodes
                        .get(helper_index - 1)
                        .ok_or(IncrementalMerkleProofError::InvalidPreviousProof)?
                } else {
                    current_level
                        .get(current_position - level_leftmost)
                        .ok_or(IncrementalMerkleProofError::InvalidChunkSize)?
                };

                // End of level?
                if current_position + 1 >= level_len as usize {
                    // Propagate dependence on proof nodes to the next layer.
                    if current_proof_nodes.contains(current_position - level_offset) {
                        // We get the offset by dividing by 2.
                        // However, if we are currently in the right side of a branch on the next level,
                        // we want the offset to reflect that.
                        let next_proof_offset = level_offset / 4 * 2;
                        next_proof_nodes.insert(current_position / 2 - next_proof_offset);
                    }

                    next_level.push(left_hash.clone());
                    break;
                }

                let right_hash = if current_position + 1 > level_rightmost {
                    proof_index += 1;
                    // Remember we used a proof node.
                    current_proof_nodes.insert(current_position + 1 - level_offset);
                    self.nodes
                        .get(proof_index - 1)
                        .ok_or(IncrementalMerkleProofError::InvalidProof)?
                } else {
                    current_level
                        .get(current_position + 1 - level_leftmost)
                        .ok_or(IncrementalMerkleProofError::InvalidChunkSize)?
                };

                let next_hash = H::Builder::default()
                    .chain(left_hash)
                    .chain(right_hash)
                    .finish();

                // If the left hash doesn't depend on any proof node, but the right hash does,
                // we need to append the left hash to the helper nodes.
                if !current_proof_nodes.contains(current_position - level_offset)
                    && current_proof_nodes.contains(current_position + 1 - level_offset)
                {
                    helper_output.push(left_hash.clone());
                }

                // TODO: Check index offsets.
                // Propagate dependence on proof nodes to the next layer.
                if current_proof_nodes.contains(current_position - level_offset)
                    || current_proof_nodes.contains(current_position + 1 - level_offset)
                {
                    // We get the offset by dividing by 2.
                    // However, if we are currently in the right side of a branch on the next level,
                    // we want the offset to reflect that.
                    let next_proof_offset = level_offset / 4 * 2;
                    next_proof_nodes.insert(current_position / 2 - next_proof_offset);
                }

                next_level.push(next_hash);
                current_position += 2;
            }

            // Update parameters for next level.
            current_level_owned = mem::take(&mut next_level);
            current_level = &current_level_owned;
            current_proof_nodes = mem::replace(&mut next_proof_nodes, BitSet::new());

            level_leftmost /= 2;
            level_rightmost /= 2;
            level_len = level_len.div_ceil(2);
            current_position = (level_leftmost / 2) * 2;
            level_offset = current_position;
        }

        // Check that we consumed everything.
        if helper_index < helper_nodes.len() {
            return Err(IncrementalMerkleProofError::InvalidPreviousProof);
        }
        if proof_index < self.nodes.len() {
            return Err(IncrementalMerkleProofError::InvalidProof);
        }

        let root = current_level_owned
            .pop()
            .ok_or(IncrementalMerkleProofError::InvalidProof)?;
        // Check that there was only the root left.
        if !current_level_owned.is_empty() {
            return Err(IncrementalMerkleProofError::InvalidProof);
        }

        // Check root against previous root.
        if let Some(prev_root) = previous_result.map(|r| &r.root) {
            if prev_root != &root {
                return Err(IncrementalMerkleProofError::InvalidProof);
            }
        }

        Ok(IncrementalMerkleProofResult {
            root,
            helper_nodes: helper_output,
            next_index: index_offset + chunk_hashes.len(),
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn total_len(&self) -> usize {
        self.total_len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum IncrementalMerkleProofError {
    InvalidPreviousProof,
    InvalidProof,
    InvalidChunkSize,
}

pub type Blake2bIncrementalMerkleProof = IncrementalMerkleProof<Blake2bHash>;
