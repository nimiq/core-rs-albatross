use std::{io::Write, ops::Range};

use nimiq_hash::{Blake2bHash, HashOutput, Hasher, SerializeContent};
use nimiq_serde::{Deserialize, Serialize};

/// A PartialMerkleProofBuilder can construct sequentially verifiable merkle proofs for a large list of data.
/// The data can then be split into chunks and each chunk has its own (small) proof.
/// Each proof can be verified by taking the previous proof's result into account (to minimize the amount of work required).
/// This way, the large list of data can be transmitted in smaller chunks and one can incrementally verify
/// the correctness of these chunks.
pub struct PartialMerkleProofBuilder {}

impl PartialMerkleProofBuilder {
    pub fn get_proofs<H: HashOutput>(
        hashes: &[H],
        chunk_size: usize,
    ) -> Result<Vec<PartialMerkleProof<H>>, PartialMerkleProofError> {
        if chunk_size == 0 {
            return Err(PartialMerkleProofError::InvalidChunkSize);
        }
        let num_chunks = hashes.len().div_ceil(chunk_size);
        let mut proofs = vec![PartialMerkleProof::empty(hashes.len()); num_chunks];
        PartialMerkleProofBuilder::compute::<H>(hashes, chunk_size, 0..hashes.len(), &mut proofs);
        Ok(proofs)
    }

    pub fn from_values<H: HashOutput, T: SerializeContent>(
        values: &[T],
        chunk_size: usize,
    ) -> Result<Vec<PartialMerkleProof<H>>, PartialMerkleProofError> {
        let hashes: Vec<H> = values
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        PartialMerkleProofBuilder::get_proofs::<H>(&hashes, chunk_size)
    }

    fn compute<H: HashOutput>(
        hashes: &[H],
        chunk_size: usize,
        current_range: Range<usize>,
        proofs: &mut Vec<PartialMerkleProof<H>>,
    ) -> H {
        let mut hasher = H::Builder::default();

        match current_range.end - current_range.start {
            0 => {
                hasher.write_all(&[]).unwrap();
                hasher.finish()
            }
            1 => {
                let index = current_range.start;
                hashes[index].clone()
            }
            len => {
                let mid = current_range.start + len.div_ceil(2);
                let left_hash = PartialMerkleProofBuilder::compute::<H>(
                    hashes,
                    chunk_size,
                    current_range.start..mid,
                    proofs,
                );
                let right_hash = PartialMerkleProofBuilder::compute::<H>(
                    hashes,
                    chunk_size,
                    mid..current_range.end,
                    proofs,
                );
                hasher.hash(&left_hash);
                hasher.hash(&right_hash);

                // Append hashes to lower proofs.
                // All chunks that are in the current range and not equal to the right chunk,
                // require the right hash in their proof.
                // The order is as expected, since we process bottom up.
                let right_chunk = mid / chunk_size;
                for proof in proofs
                    .iter_mut()
                    .take(right_chunk)
                    .skip(current_range.start / chunk_size)
                {
                    proof.nodes.push(right_hash.clone());
                }

                hasher.finish()
            }
        }
    }
}

/// A PartialMerkleProof is a merkle proof for a single chunk from a large set of data.
/// These proofs can only be verified incrementally, i.e., one has to start with the first chunk of data.
/// The proof for the second chunk then takes as an input the result of the first chunk's proof and so on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "H: HashOutput"))]
pub struct PartialMerkleProof<H: HashOutput> {
    total_len: u32,
    nodes: Vec<H>,
}

#[derive(Debug)]
pub struct PartialMerkleProofResult<H: HashOutput> {
    /// The calculated root of the merkle proof.
    root: H,
    /// A set of hashes in the merkle tree that are used in the next proof's verification.
    helper_nodes: Vec<H>,
    /// The next index after this proof's leaf nodes in the overall data.
    next_index: usize,
}

impl<H: HashOutput> PartialMerkleProofResult<H> {
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

impl<H: HashOutput> PartialMerkleProof<H> {
    pub fn empty(total_len: usize) -> Self {
        PartialMerkleProof {
            total_len: total_len as u32,
            nodes: Vec::new(),
        }
    }

    pub fn compute_root_from_values<T: SerializeContent>(
        &self,
        chunk_values: &[T],
        previous_result: Option<&PartialMerkleProofResult<H>>,
    ) -> Result<PartialMerkleProofResult<H>, PartialMerkleProofError> {
        let hashes: Vec<H> = chunk_values
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        self.compute_root(&hashes, previous_result)
    }

    pub fn compute_root(
        &self,
        chunk_hashes: &[H],
        previous_result: Option<&PartialMerkleProofResult<H>>,
    ) -> Result<PartialMerkleProofResult<H>, PartialMerkleProofError> {
        let index_offset = previous_result.map(|r| r.next_index).unwrap_or(0);
        let last_index = chunk_hashes.len() + index_offset;
        let empty_vec = Vec::new();
        let helper_nodes = previous_result
            .map(|r| &r.helper_nodes)
            .unwrap_or(&empty_vec);
        let mut helper_index = helper_nodes.len();
        let mut proof_index = 0;

        let mut helper_output = Vec::new();
        let (_, root) = self.compute(
            chunk_hashes,
            0..self.total_len(),
            index_offset,
            helper_nodes,
            &mut helper_index,
            &mut proof_index,
            &mut helper_output,
        )?;

        // Check that we consumed everything.
        if helper_index > 0 {
            return Err(PartialMerkleProofError::InvalidPreviousProof);
        }
        if proof_index < self.nodes.len() {
            return Err(PartialMerkleProofError::InvalidProof);
        }

        Ok(PartialMerkleProofResult {
            root,
            helper_nodes: helper_output,
            next_index: last_index,
        })
    }

    fn compute(
        &self,
        hashes: &[H],
        current_range: Range<usize>,
        index_offset: usize,
        helper_nodes: &[H],
        helper_index: &mut usize,
        proof_index: &mut usize,
        helper_output: &mut Vec<H>,
    ) -> Result<(bool, H), PartialMerkleProofError> {
        let mut hasher = H::Builder::default();

        // Catch cases that require proof/helper nodes.
        if current_range.end <= index_offset {
            *helper_index -= 1;
            let hash = helper_nodes
                .get(*helper_index)
                .ok_or(PartialMerkleProofError::InvalidPreviousProof)?;
            return Ok((false, hash.clone()));
        }
        if current_range.start >= index_offset + hashes.len() {
            let hash = self
                .nodes
                .get(*proof_index)
                .ok_or(PartialMerkleProofError::InvalidProof)?;
            *proof_index += 1;
            return Ok((true, hash.clone()));
        }

        match current_range.end - current_range.start {
            0 => {
                hasher.write_all(&[]).unwrap();
                Ok((false, hasher.finish()))
            }
            1 => {
                // This should always work as we catch helper/proof nodes above.
                Ok((false, hashes[current_range.start - index_offset].clone()))
            }
            len => {
                let mid = current_range.start + len.div_ceil(2);
                let (proof_node_left, left_hash) = self.compute(
                    hashes,
                    current_range.start..mid,
                    index_offset,
                    helper_nodes,
                    helper_index,
                    proof_index,
                    helper_output,
                )?;
                let (proof_node_right, right_hash) = self.compute(
                    hashes,
                    mid..current_range.end,
                    index_offset,
                    helper_nodes,
                    helper_index,
                    proof_index,
                    helper_output,
                )?;
                hasher.hash(&left_hash);
                hasher.hash(&right_hash);

                // If there is no proof node in the left branch, but one in the right branch,
                // we need to remember the LHS for the next proof to verify.
                if !proof_node_left && proof_node_right {
                    helper_output.push(left_hash);
                }

                Ok((proof_node_left || proof_node_right, hasher.finish()))
            }
        }
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
pub enum PartialMerkleProofError {
    InvalidPreviousProof,
    InvalidProof,
    InvalidChunkSize,
}

pub type Blake2bPartialMerkleProof = PartialMerkleProof<Blake2bHash>;
