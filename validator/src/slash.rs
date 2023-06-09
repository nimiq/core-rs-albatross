use std::collections::HashSet;

use nimiq_block::{Block, ForkProof, MacroBlock, MacroHeader, MicroBlock};
use nimiq_serde::Serialize;

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

    /// Applies a block to the pool, removing processed fork proofs.
    pub fn apply_block(&mut self, block: &Block) {
        match block {
            Block::Micro(MicroBlock {
                body: Some(extrinsics),
                ..
            }) => {
                for fork_proof in extrinsics.fork_proofs.iter() {
                    self.fork_proofs.remove(fork_proof);
                }
            }
            Block::Macro(MacroBlock {
                header: MacroHeader { block_number, .. },
                ..
            }) => {
                // After a macro block, remove all fork proofs that would not be valid anymore
                // from now on.
                self.fork_proofs
                    .retain(|proof| proof.is_valid_at(*block_number + 1));
            }
            _ => {}
        }
    }

    /// Reverts a block, re-adding fork proofs.
    pub fn revert_block(&mut self, block: &Block) {
        if let Block::Micro(MicroBlock {
            body: Some(extrinsics),
            ..
        }) = block
        {
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
            let proof_len = proof.serialized_size();
            if size + proof_len < max_size {
                proofs.push(proof.clone());
                size += proof_len;
            }
        }
        proofs
    }
}
