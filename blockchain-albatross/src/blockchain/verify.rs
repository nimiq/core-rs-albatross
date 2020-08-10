use parking_lot::MappedRwLockReadGuard;

use block::{BlockError, BlockHeader, BlockJustification, BlockType, ViewChange};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use bls::PublicKey;
use database::Transaction;
use primitives::policy;

use crate::{Blockchain, PushError};

/// Verifies blocks
impl Blockchain {
    pub fn verify_block_header(
        &self,
        header: &BlockHeader,
        intended_slot_owner: &MappedRwLockReadGuard<PublicKey>,
        txn_opt: Option<&Transaction>,
    ) -> Result<(), PushError> {
        // Check the version
        if header.version() != policy::VERSION {
            warn!("Rejecting block - wrong version");
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_info_opt = self
            .chain_store
            .get_chain_info(&header.parent_hash(), false, txn_opt);

        if prev_info_opt.is_none() {
            warn!("Rejecting block - unknown predecessor");
            return Err(PushError::Orphan);
        }

        // Check that the block is a valid successor of its predecessor.
        let prev_info = prev_info_opt.unwrap();

        if self.get_next_block_type(Some(prev_info.head.block_number())) != header.ty() {
            warn!("Rejecting block - wrong block type ({:?})", header.ty());
            return Err(PushError::InvalidSuccessor);
        }

        // Check the block number
        if prev_info.head.block_number() + 1 != header.block_number() {
            warn!(
                "Rejecting block - wrong block number ({:?})",
                header.block_number()
            );
            return Err(PushError::InvalidSuccessor);
        }

        // Check that the current block timestamp is equal or greater than the timestamp of the
        // previous block.
        if prev_info.head.timestamp() >= header.timestamp() {
            warn!("Rejecting block - block timestamp precedes parent timestamp");
            return Err(PushError::InvalidSuccessor);
        }

        // Check that the current block timestamp less the node's current time is less than or equal
        // to the allowed maximum drift. Basically, we check that the block isn't from the future.
        // Both times are given in Unix time standard in millisecond precision.
        if header.timestamp().saturating_sub(self.time.now()) > policy::TIMESTAMP_MAX_DRIFT {
            warn!("Rejecting block - block timestamp exceeds allowed maximum drift");
            return Err(PushError::InvalidBlock(BlockError::FromTheFuture));
        }

        // Check if the seed was signed by the intended producer.
        if let Err(e) = header
            .seed()
            .verify(prev_info.head.seed(), intended_slot_owner)
        {
            warn!("Rejecting block - invalid seed ({:?})", e);
            return Err(PushError::InvalidBlock(BlockError::InvalidSeed));
        }

        if header.ty() == BlockType::Macro {
            // Check if the parent election hash matches the current election head hash
            if header.parent_election_hash().unwrap() != &self.state().election_head_hash {
                warn!("Rejecting block - wrong parent election hash");
                return Err(PushError::InvalidSuccessor);
            }
        }

        Ok(())
    }

    pub fn verify_block_justification(
        &self,
        header: &BlockHeader,
        justification_opt: &Option<BlockJustification>,
        intended_slot_owner: &MappedRwLockReadGuard<PublicKey>,
        txn_opt: Option<&Transaction>,
    ) -> Result<(), PushError> {
        if let None = justification_opt {
            return Err(PushError::InvalidBlock(BlockError::NoJustification));
        }

        if let Some(BlockJustification::Micro(justification)) = justification_opt {
            // Verify signature
            let signature = justification.signature.uncompress();

            if let Err(e) = signature {
                warn!("Rejecting block - invalid signature ({:?})", e);
                return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
            }

            if !intended_slot_owner.verify(header, &signature.unwrap()) {
                warn!("Rejecting block - invalid signature for intended slot owner");

                debug!("Block hash: {}", header.hash());

                debug!("Intended slot owner: {:?}", intended_slot_owner.compress());

                return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
            }

            // Check if a view change occurred - if so, validate the proof
            let prev_info = self
                .chain_store
                .get_chain_info(&header.parent_hash(), false, txn_opt)
                .unwrap();

            let view_number = if policy::is_macro_block_at(header.block_number() - 1) {
                // Reset view number in new batch
                0
            } else {
                prev_info.head.view_number()
            };

            let new_view_number = header.view_number();

            if new_view_number < view_number {
                warn!(
                    "Rejecting block - lower view number {:?} < {:?}",
                    header.view_number(),
                    view_number
                );
                return Err(PushError::InvalidBlock(BlockError::InvalidViewNumber));
            } else if new_view_number == view_number && justification.view_change_proof.is_some() {
                warn!("Rejecting block - must not contain view change proof");
                return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
            } else if new_view_number > view_number && justification.view_change_proof.is_none() {
                warn!("Rejecting block - missing view change proof");
                return Err(PushError::InvalidBlock(BlockError::NoViewChangeProof));
            } else if new_view_number > view_number && justification.view_change_proof.is_some() {
                let view_change = ViewChange {
                    block_number: header.block_number(),
                    new_view_number: header.view_number(),
                    prev_seed: prev_info.head.seed().clone(),
                };

                if let Err(e) = justification.view_change_proof.as_ref().unwrap().verify(
                    &view_change,
                    &self.current_validators(),
                    policy::TWO_THIRD_SLOTS,
                ) {
                    warn!("Rejecting block - bad view change proof: {:?}", e);
                    return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                }
            }
        }

        if let Some(BlockJustification::Macro(justification)) = justification_opt {
            // Verify PBFT proof.
            if let Err(e) = justification.verify(
                header.hash(),
                &self.current_validators(),
                policy::TWO_THIRD_SLOTS,
            ) {
                warn!(
                    "Rejecting block - macro block with bad justification: {}",
                    e
                );
                return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
            }
        }

        Ok(())
    }

    // TODO: Move this check to someplace else!
    // // In case of an election block make sure it contains validators
    // if policy::is_election_block_at(header.block_number)
    //     && body.unwrap_macro().validators.is_none()
    // {
    //     return Err(PushError::InvalidSuccessor);
    // }

    // TODO: Move this check to someplace else!
    // let history_root = self
    //     .get_history_root(policy::batch_at(header.block_number), txn_opt)
    //     .ok_or(PushError::BlockchainError(
    //         BlockchainError::FailedLoadingMainChain,
    //     ))?;
    // if body.unwrap_macro().history_root != history_root {
    //     warn!("Rejecting block - wrong history root");
    //     return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
    // }

    // TODO: Move this check to someplace else!
    // // The macro body cannot be None.
    // if let Some(ref body) = macro_block.body {
    // let body_hash: Blake2bHash = body.hash();
    // if body_hash != macro_block.header.body_root {
    // warn!("Rejecting block - Header body hash doesn't match real body hash");
    // return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
    //      }
    // }

    // // TODO: This should be moved to verify.rs
    // // Validate fork proofs
    // for fork_proof in &micro_block.body.as_ref().unwrap().fork_proofs {
    // // NOTE: if this returns None, that means that at least the previous block doesn't
    // // exist, so that fork proof is invalid anyway.
    // let (slot, _) = self
    // .get_slot_at(
    // fork_proof.header1.block_number,
    // fork_proof.header1.view_number,
    // Some(&read_txn),
    // )
    // .ok_or(PushError::InvalidSuccessor)?;
    //
    // if fork_proof
    // .verify(&slot.public_key().uncompress_unchecked())
    // .is_err()
    // {
    // warn!("Rejecting block - Bad fork proof: invalid owner signature");
    // return Err(PushError::InvalidSuccessor);
    // }}
}
