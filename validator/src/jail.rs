use std::collections::HashSet;

use nimiq_block::{Block, EquivocationProof, MacroBlock, MacroHeader, MicroBlock};
use nimiq_serde::Serialize;

/// Pool for holding distinct equivocation proofs that haven't been seen in blocks yet.
#[derive(Default)]
pub struct EquivocationProofPool {
    equivocation_proofs: HashSet<EquivocationProof>,
}

impl EquivocationProofPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an equivocation proof if it is not yet part of the pool.
    /// Returns whether it has been added.
    pub fn insert(&mut self, equivocation_proof: EquivocationProof) -> bool {
        self.equivocation_proofs.insert(equivocation_proof)
    }

    /// Applies a block to the pool, removing processed equivocation proofs.
    pub fn apply_block(&mut self, block: &Block) {
        match block {
            Block::Micro(MicroBlock {
                body: Some(extrinsics),
                ..
            }) => {
                for equivocation_proof in extrinsics.equivocation_proofs.iter() {
                    self.equivocation_proofs.remove(equivocation_proof);
                }
            }
            Block::Macro(MacroBlock {
                header: MacroHeader { block_number, .. },
                ..
            }) => {
                // After a macro block, remove all equivocation proofs that would not be valid anymore
                // from now on.
                self.equivocation_proofs
                    .retain(|proof| proof.is_valid_at(*block_number + 1));
            }
            _ => {}
        }
    }

    /// Reverts a block, re-adding equivocation proofs.
    pub fn revert_block(&mut self, block: &Block) {
        if let Block::Micro(MicroBlock {
            body: Some(extrinsics),
            ..
        }) = block
        {
            for equivocation_proof in extrinsics.equivocation_proofs.iter() {
                self.equivocation_proofs.insert(equivocation_proof.clone());
            }
        }
    }

    /// Returns a list of current equivocation proofs.
    pub fn get_equivocation_proofs_for_block(&self, max_size: usize) -> Vec<EquivocationProof> {
        let mut proofs = Vec::new();
        let mut size = 0;
        for proof in self.equivocation_proofs.iter() {
            let proof_len = proof.serialized_size();
            if size + proof_len < max_size {
                proofs.push(proof.clone());
                size += proof_len;
            }
        }
        proofs
    }
}
