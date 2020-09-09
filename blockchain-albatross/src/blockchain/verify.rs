use parking_lot::MappedRwLockReadGuard;

#[cfg(feature = "metrics")]

use crate::hash::{Blake2bHash, Hash};
use block::{
    Block, BlockBody, BlockError, BlockHeader, BlockJustification, BlockType, ForkProof, ViewChange,
};
use bls::PublicKey;
use database::Transaction as DBtx;
use primitives::policy;
use transaction::Transaction;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
use crate::{Blockchain, BlockchainError, PushError};
use std::cmp::Ordering;

/// Implements methods to verify the validity of blocks.
impl Blockchain {
    /// Verifies the header of a block.
    pub fn verify_block_header(
        &self,
        header: &BlockHeader,
        intended_slot_owner: &MappedRwLockReadGuard<PublicKey>,
        txn_opt: Option<&DBtx>,
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

    /// Verifies the justification of a block.
    pub fn verify_block_justification(
        &self,
        header: &BlockHeader,
        justification_opt: &Option<BlockJustification>,
        intended_slot_owner: &MappedRwLockReadGuard<PublicKey>,
        txn_opt: Option<&DBtx>,
    ) -> Result<(), PushError> {
        // Checks if the justification exists.
        if let None = justification_opt {
            return Err(PushError::InvalidBlock(BlockError::NoJustification));
        }

        // If the block is a macro block, verify the PBFT proof.
        if let Some(BlockJustification::Macro(justification)) = justification_opt {
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

        // If the block is a micro block, verify the signature and the view changes.
        if let Some(BlockJustification::Micro(justification)) = justification_opt {
            // Verify the signature on the justification.
            let signature = justification.signature.uncompress();

            if let Err(e) = signature {
                warn!("Rejecting block - invalid signature ({:?})", e);
                return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
            }

            if !intended_slot_owner.verify_hash(header.hash_blake2s(), &signature.unwrap()) {
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

        Ok(())
    }

    /// Verifies the body of a block. This does not check anything that depends on the state. Ex: If
    /// an account has enough funds.
    pub fn verify_block_body(
        &self,
        header: &BlockHeader,
        body_opt: &Option<BlockBody>,
        txn_opt: Option<&DBtx>,
    ) -> Result<(), PushError> {
        // Checks if the body exists.
        if let None = body_opt {
            return Err(PushError::InvalidBlock(BlockError::MissingBody));
        }

        if let Some(BlockBody::Macro(body)) = body_opt {
            // Check the body root.
            if &body.hash::<Blake2bHash>() != header.body_root() {
                warn!("Rejecting block - Header body hash doesn't match real body hash");
                return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
            }

            // In case of an election block make sure it contains validators.
            if policy::is_election_block_at(header.block_number()) && body.validators.is_none() {
                return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
            }

            let history_root = self
                .get_history_root(policy::epoch_at(header.block_number()), txn_opt)
                .ok_or(PushError::BlockchainError(
                    BlockchainError::FailedLoadingMainChain,
                ))?;

            if body.history_root != history_root {
                warn!("Rejecting block - wrong history root");
                return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
            }
        }

        if let Some(BlockBody::Micro(body)) = body_opt {
            // Check the body root.
            if &body.hash::<Blake2bHash>() != header.body_root() {
                warn!("Rejecting block - Header body hash doesn't match real body hash");
                return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
            }

            // Validate the fork proofs.
            let mut previous_proof: Option<&ForkProof> = None;

            for proof in &body.fork_proofs {
                // Ensure proofs are ordered and unique.
                if let Some(previous) = previous_proof {
                    match previous.cmp(&proof) {
                        Ordering::Equal => {
                            return Err(PushError::InvalidBlock(BlockError::DuplicateForkProof));
                        }
                        Ordering::Greater => {
                            return Err(PushError::InvalidBlock(BlockError::ForkProofsNotOrdered));
                        }
                        _ => (),
                    }
                }

                // Check that the proof is within the reporting window.
                if !proof.is_valid_at(header.block_number()) {
                    return Err(PushError::InvalidBlock(BlockError::InvalidForkProof));
                }

                // Get intended slot owner for that block.
                let (slot, _) = self.get_slot_owner_at(
                    proof.header1.block_number,
                    proof.header1.view_number,
                    txn_opt,
                );

                // Verify fork proof.
                if proof
                    .verify(&slot.public_key().uncompress_unchecked())
                    .is_err()
                {
                    warn!("Rejecting block - Bad fork proof: invalid owner signature");
                    return Err(PushError::InvalidBlock(BlockError::InvalidForkProof));
                }

                previous_proof = Some(proof);
            }

            // Verify transactions.
            let mut previous_tx: Option<&Transaction> = None;

            for tx in &body.transactions {
                // Ensure transactions are ordered and unique.
                if let Some(previous) = previous_tx {
                    match previous.cmp_block_order(tx) {
                        Ordering::Equal => {
                            return Err(PushError::InvalidBlock(BlockError::DuplicateTransaction));
                        }
                        Ordering::Greater => {
                            return Err(PushError::InvalidBlock(
                                BlockError::TransactionsNotOrdered,
                            ));
                        }
                        _ => (),
                    }
                }

                // Check that the transaction is within its validity window.
                if !tx.is_valid_at(header.block_number()) {
                    return Err(PushError::InvalidBlock(BlockError::ExpiredTransaction));
                }

                // Check intrinsic transaction invariants.
                if let Err(e) = tx.verify(self.network_id) {
                    return Err(PushError::InvalidBlock(BlockError::InvalidTransaction(e)));
                }

                previous_tx = Some(tx);
            }
        }

        Ok(())
    }

    /// Verifies the state of a block.
    pub fn verify_block_state(
        &self,
        state: &BlockchainState,
        chain_info: &ChainInfo,
        txn_opt: Option<&DBtx>,
    ) -> Result<(), PushError> {
        let accounts = &state.accounts;

        let block = &chain_info.head;

        // Verify accounts hash.
        let accounts_hash = accounts.hash(txn_opt);

        trace!("Block state root: {}", block.state_root());

        trace!("Accounts hash:    {}", accounts_hash);

        if block.state_root() != &accounts_hash {
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        // For macro blocks we have additional checks.
        if let Block::Macro(macro_block) = block {
            let staking_contract = self.get_staking_contract();

            if staking_contract.previous_lost_rewards()
                != macro_block.body.as_ref().unwrap().lost_reward_set
            {
                warn!("Rejecting block - Lost rewards set doesn't match real lost rewards set");
                return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
            }

            if staking_contract.previous_disabled_slots()
                != macro_block.body.as_ref().unwrap().disabled_set
            {
                warn!("Rejecting block - Disabled set doesn't match real disabled set");
                return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
            }

            if macro_block.is_election_block() {
                let real_validators = &self.next_slots(&macro_block.header.seed).validator_slots;

                let block_validators = macro_block
                    .body
                    .as_ref()
                    .unwrap()
                    .validators
                    .as_ref()
                    .unwrap();

                if real_validators != block_validators {
                    warn!("Rejecting block - Validators don't match real validators");
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }
            }
        }

        Ok(())
    }
}
