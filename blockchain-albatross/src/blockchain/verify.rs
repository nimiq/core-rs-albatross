use parking_lot::MappedRwLockReadGuard;

use block::{BlockError, BlockHeader, ViewChange, ViewChangeProof};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use bls::PublicKey;
use database::Transaction;
use primitives::policy;

use crate::{Blockchain, OptionalCheck, PushError};

// complicated stuff
impl Blockchain {
    pub fn verify_block_header(
        &self,
        header: &BlockHeader,
        view_change_proof: OptionalCheck<&ViewChangeProof>,
        intended_slot_owner: &MappedRwLockReadGuard<PublicKey>,
        txn_opt: Option<&Transaction>,
    ) -> Result<(), PushError> {
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
        // Both times are given in Unix time standard.
        if header.timestamp().saturating_sub(self.time.now()) > policy::TIMESTAMP_MAX_DRIFT {
            warn!("Rejecting block - block timestamp exceeds allowed maximum drift");
            return Err(PushError::InvalidBlock(BlockError::FromTheFuture));
        }

        // Check if a view change occurred - if so, validate the proof
        let view_number = if policy::is_macro_block_at(header.block_number() - 1) {
            0 // Reset view number in new batch
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
        } else if new_view_number > view_number {
            match view_change_proof {
                OptionalCheck::None => {
                    warn!("Rejecting block - missing view change proof");
                    return Err(PushError::InvalidBlock(BlockError::NoViewChangeProof));
                }
                OptionalCheck::Some(ref view_change_proof) => {
                    let view_change = ViewChange {
                        block_number: header.block_number(),
                        new_view_number: header.view_number(),
                        prev_seed: prev_info.head.seed().clone(),
                    };
                    if let Err(e) = view_change_proof.verify(
                        &view_change,
                        &self.current_validators(),
                        policy::TWO_THIRD_SLOTS,
                    ) {
                        warn!("Rejecting block - bad view change proof: {:?}", e);
                        return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                    }
                }
                OptionalCheck::Skip => {}
            }
        } else if new_view_number == view_number && view_change_proof.is_some() {
            warn!("Rejecting block - must not contain view change proof");
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        // Check if the block was produced (and signed) by the intended producer
        if let Err(e) = header
            .seed()
            .verify(prev_info.head.seed(), intended_slot_owner)
        {
            warn!("Rejecting block - invalid seed ({:?})", e);
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        if let BlockHeader::Macro(ref header) = header {
            // TODO: Move this check to someplace else!
            // // In case of an election block make sure it contains validators
            // if policy::is_election_block_at(header.block_number)
            //     && body.unwrap_macro().validators.is_none()
            // {
            //     return Err(PushError::InvalidSuccessor);
            // }

            // Check if the parent election hash matches the current election head hash
            if header.parent_election_hash != self.state().election_head_hash {
                warn!("Rejecting block - wrong parent election hash");
                return Err(PushError::InvalidSuccessor);
            }

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
        }

        Ok(())
    }
}
