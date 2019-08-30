use block_albatross::{ForkProof, Block, MicroBlock};
use std::collections::HashSet;
use beserial::Serialize;

#[derive(Default)]
pub struct ForkProofPool {
    fork_proofs: HashSet<ForkProof>,
}

impl ForkProofPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a fork proof if it is not yet part of the pool.
    /// Returns whether it has been added.
    pub fn insert(&mut self, fork_proof: ForkProof) -> bool {
        self.fork_proofs.insert(fork_proof)
    }

    /// Checks whether a fork proof is already part of the pool.
    pub fn contains(&self, fork_proof: &ForkProof) -> bool {
        self.fork_proofs.contains(fork_proof)
    }

    /// Applies a block to the pool, removing processed fork proofs.
    pub fn apply_block(&mut self, block: &Block) {
        if let Block::Micro(MicroBlock { extrinsics: Some(extrinsics), .. }) = block {
            for fork_proof in extrinsics.fork_proofs.iter() {
                self.fork_proofs.remove(fork_proof);
            }
        }
    }

    /// Reverts a block, re-adding fork proofs.
    pub fn revert_block(&mut self, block: &Block) {
        if let Block::Micro(MicroBlock { extrinsics: Some(extrinsics), .. }) = block {
            for fork_proof in extrinsics.fork_proofs.iter() {
                self.fork_proofs.insert(fork_proof.clone());
            }
        }
    }

    /// Returns a list of current fork proofs.
    pub fn get_fork_proofs_for_block(&self, max_size: usize) -> Vec<ForkProof> {
        let mut proofs = Vec::new();
        let mut size = 0;
        for proof in self.fork_proofs.iter() {
            if size + proof.serialized_size() < max_size {
                proofs.push(proof.clone());
                size += proof.serialized_size();
            }
        }
        proofs
    }
}
