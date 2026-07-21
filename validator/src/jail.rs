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
                header:
                    MacroHeader {
                        block_number,
                        version,
                        ..
                    },
                ..
            }) => {
                // After a macro block, remove all equivocation proofs that would not be valid anymore
                // from now on.
                self.equivocation_proofs
                    .retain(|proof| proof.is_valid_at(*block_number + 1, *version));
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

    /// Returns a list of equivocation proofs that are valid for inclusion in a block
    /// with the given block number and version.
    pub fn get_equivocation_proofs_for_block(
        &self,
        block_number: u32,
        version: u16,
        max_size: usize,
    ) -> Vec<EquivocationProof> {
        let mut proofs = Vec::new();
        let mut size = 0;
        for proof in self.equivocation_proofs.iter() {
            // The pool is only pruned at macro blocks and proofs are re-added on reverts
            // unconditionally, so proofs can expire mid-batch. Mirror the check done by
            // `MicroBlock` body verification to never produce a block that includes an
            // expired proof, which the network would reject
            if !proof.is_valid_at(block_number, version) {
                continue;
            }
            let proof_len = proof.serialized_size();
            if size + proof_len < max_size {
                proofs.push(proof.clone());
                size += proof_len;
            }
        }
        proofs
    }
}

#[cfg(test)]
mod tests {
    use nimiq_block::{ForkProof, MicroHeader};
    use nimiq_hash::HashOutput;
    use nimiq_keys::{Address, KeyPair, PrivateKey};
    use nimiq_primitives::policy::Policy;

    use super::*;

    fn equivocation_proof(offense_block: u32) -> EquivocationProof {
        // The pool never verifies signatures, so any key works here.
        let key = KeyPair::from(PrivateKey::from_bytes(&[1u8; 32]).unwrap());
        let header1 = MicroHeader {
            version: 1,
            block_number: offense_block,
            timestamp: 0,
            ..Default::default()
        };
        let mut header2 = header1.clone();
        header2.timestamp = 1;
        let justification1 = key.sign(header1.hash().as_bytes());
        let justification2 = key.sign(header2.hash().as_bytes());
        ForkProof::new(
            Address::burn_address(),
            header1,
            justification1,
            header2,
            justification2,
        )
        .into()
    }

    #[test]
    fn get_equivocation_proofs_for_block_filters_expired_proofs() {
        let offense_block = Policy::genesis_block_number();
        let mut pool = EquivocationProofPool::new();
        assert!(pool.insert(equivocation_proof(offense_block)));

        let last_reportable = Policy::last_block_of_equivocation_reporting_window(offense_block);
        assert!(last_reportable < Policy::last_block_of_collateral_lockup(offense_block));

        // Within the reporting window the proof is returned.
        assert_eq!(
            pool.get_equivocation_proofs_for_block(last_reportable, 2, usize::MAX)
                .len(),
            1
        );
        // Past the window it is filtered out from version 2 on.
        assert!(pool
            .get_equivocation_proofs_for_block(last_reportable + 1, 2, usize::MAX)
            .is_empty());
        // Legacy versions still use the longer window.
        assert_eq!(
            pool.get_equivocation_proofs_for_block(last_reportable + 1, 1, usize::MAX)
                .len(),
            1
        );
    }

    #[test]
    fn get_equivocation_proofs_for_block_keeps_valid_drops_expired_together() {
        let older = Policy::genesis_block_number();
        // An offense one block later has a reporting window that ends one block later.
        let newer = older + 1;
        let mut pool = EquivocationProofPool::new();
        assert!(pool.insert(equivocation_proof(older)));
        assert!(pool.insert(equivocation_proof(newer)));

        // A block past `older`'s window but still within `newer`'s window: the two proofs
        // must be filtered independently, not all-or-nothing.
        let at = Policy::last_block_of_equivocation_reporting_window(older) + 1;
        let returned = pool.get_equivocation_proofs_for_block(at, 2, usize::MAX);
        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].block_number(), newer);
    }
}
