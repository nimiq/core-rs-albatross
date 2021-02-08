use std::cmp::Ordering;

use block::{Block, BlockBody, BlockError, BlockHeader, BlockJustification, BlockType, ForkProof, MacroBody, ViewChange};
use bls::PublicKey;
use database::Transaction as DBtx;
use primitives::policy;
use transaction::Transaction;

use crate::blockchain_state::BlockchainState;
use crate::hash::{Blake2bHash, Hash};
use crate::{Blockchain, PushError};

/// Implements methods to verify the validity of blocks.
impl Blockchain {
    /// Verifies the header of a block. This function is used when we are pushing a normal block
    /// into the chain. It cannot be used when syncing, since this performs more strict checks than
    /// the ones we make when syncing.
    /// This only performs checks that can be made BEFORE the state is updated with the block. All
    /// checks that require the updated state (ex: if an account has enough funds) are made on the
    /// verify_block_state method.
    pub fn verify_block_header(&self, header: &BlockHeader, intended_slot_owner: &PublicKey, txn_opt: Option<&DBtx>) -> Result<(), PushError> {
        // Check the version
        if header.version() != policy::VERSION {
            warn!("Rejecting block - wrong version");
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_info = self.chain_store.get_chain_info(&header.parent_hash(), false, txn_opt).unwrap();

        // Check that the block is a valid successor of its predecessor.
        if self.get_next_block_type(Some(prev_info.head.block_number())) != header.ty() {
            warn!("Rejecting block - wrong block type ({:?})", header.ty());
            return Err(PushError::InvalidSuccessor);
        }

        // Check the block number
        if prev_info.head.block_number() + 1 != header.block_number() {
            warn!("Rejecting block - wrong block number ({:?})", header.block_number());
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
        if let Err(e) = header.seed().verify(prev_info.head.seed(), intended_slot_owner) {
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
        intended_slot_owner: &PublicKey,
        txn_opt: Option<&DBtx>,
    ) -> Result<(), PushError> {
        // Checks if the justification exists. If yes, unwrap it.
        let justification = justification_opt.as_ref().ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

        match justification {
            BlockJustification::Micro(justification) => {
                // If the block is a micro block, verify the signature and the view changes.
                let signature = justification.signature.uncompress();

                if let Err(e) = signature {
                    warn!("Rejecting block - invalid signature ({:?})", e);
                    return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                }

                // Verify the signature on the justification.
                if !intended_slot_owner.verify_hash(header.hash_blake2s(), &signature.unwrap()) {
                    warn!("Rejecting block - invalid signature for intended slot owner");

                    debug!("Block hash: {}", header.hash());

                    debug!("Intended slot owner: {:?}", intended_slot_owner.compress());
                    return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                }

                // Check if a view change occurred - if so, validate the proof
                let prev_info = self.chain_store.get_chain_info(&header.parent_hash(), false, txn_opt).unwrap();

                let view_number = if policy::is_macro_block_at(header.block_number() - 1) {
                    // Reset view number in new batch
                    0
                } else {
                    prev_info.head.view_number()
                };

                let new_view_number = header.view_number();

                if new_view_number < view_number {
                    warn!("Rejecting block - lower view number {:?} < {:?}", header.view_number(), view_number);
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

                    if !justification
                        .view_change_proof
                        .as_ref()
                        .unwrap()
                        .verify(&view_change, &self.current_validators())
                    {
                        warn!("Rejecting block - bad view change proof");
                        return Err(PushError::InvalidBlock(BlockError::InvalidViewChangeProof));
                    }
                }
            }
            BlockJustification::Macro(justification) => {
                // If the block is a macro block, verify the Tendermint proof.
                if !justification.verify(header.hash(), header.block_number(), &self.current_validators()) {
                    warn!("Rejecting block - macro block with bad justification");
                    return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                }
            }
        }

        Ok(())
    }

    /// Verifies the body of a block.
    /// This only performs checks that can be made BEFORE the state is updated with the block. All
    /// checks that require the updated state (ex: if an account has enough funds) are made on the
    /// verify_block_state method.
    pub fn verify_block_body(&self, header: &BlockHeader, body_opt: &Option<BlockBody>, txn_opt: Option<&DBtx>) -> Result<(), PushError> {
        // Checks if the body exists. If yes, unwrap it.
        let body = body_opt.as_ref().ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        match body {
            BlockBody::Micro(body) => {
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
                    let (slot, _) = self.get_slot_owner_at(proof.header1.block_number, proof.header1.view_number, txn_opt);

                    // Verify fork proof.
                    if proof.verify(&slot.public_key().uncompress_unchecked()).is_err() {
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
                                return Err(PushError::InvalidBlock(BlockError::TransactionsNotOrdered));
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
            BlockBody::Macro(body) => {
                // Check the body root.
                if &body.hash::<Blake2bHash>() != header.body_root() {
                    warn!("Rejecting block - Header body hash doesn't match real body hash");
                    return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
                }

                // In case of an election block make sure it contains validators, if it is not an
                // election block make sure it doesn't.
                if policy::is_election_block_at(header.block_number()) != body.validators.is_some() {
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }
            }
        }

        Ok(())
    }

    /// Verifies a block against the blockchain state AFTER it gets updated with the block (ex: if
    /// an account has enough funds).
    /// It receives a block as input but that block is only required to have a header (the body and
    /// justification are optional, we don't need them).
    /// In the case of a macro block the checks we perform vary a little depending if we provide a
    /// block with a body:
    /// - With body: we check each field of the body against the same field calculated from our
    ///   current state.
    /// - Without body: we construct a body using fields calculated from our current state and
    ///   compare its hash with the body hash in the header. In this case we return the calculated
    ///   body.
    pub fn verify_block_state(&self, state: &BlockchainState, block: &Block, txn_opt: Option<&DBtx>) -> Result<Option<MacroBody>, PushError> {
        let accounts = &state.accounts;

        // Verify accounts hash.
        let accounts_hash = accounts.hash(txn_opt);

        trace!("Block state root: {}", block.state_root());

        trace!("Accounts hash:    {}", accounts_hash);

        if block.state_root() != &accounts_hash {
            error!("State: expected {:?}, found {:?}", block.state_root(), accounts_hash);
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        // For macro blocks we have additional checks. We simply construct what the body should be
        // from our own state and then compare it with the body hash in the header.
        if let Block::Macro(macro_block) = block {
            // Get the history root.
            let real_history_root = match self
                .history_store
                .get_history_tree_root(policy::epoch_at(macro_block.header.block_number), txn_opt)
            {
                Some(hash) => hash,
                None => {
                    return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
                }
            };

            // Get the lost rewards and disabled sets.
            let staking_contract = self.get_staking_contract();

            let real_lost_rewards = staking_contract.previous_lost_rewards();

            let real_disabled_slots = staking_contract.previous_disabled_slots();

            // Get the validators.
            let real_validators = if macro_block.is_election_block() {
                Some(self.next_slots(&macro_block.header.seed).validator_slots)
            } else {
                None
            };

            // Check the real values against the block.
            if let Some(body) = &macro_block.body {
                // If we were given a body, then check each value against the corresponding value in
                // the body.
                if real_history_root != body.history_root {
                    warn!("Rejecting block - History root doesn't match real history root");
                    return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
                }

                if real_lost_rewards != body.lost_reward_set {
                    warn!("Rejecting block - Lost rewards set doesn't match real lost rewards set");
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_disabled_slots != body.disabled_set {
                    warn!("Rejecting block - Disabled set doesn't match real disabled set");
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_validators != body.validators {
                    warn!("Rejecting block - Validators don't match real validators");
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }
            } else {
                // If we were not given a body, then we construct a body from our values and check
                // its hash against the block header.
                let real_body = MacroBody {
                    validators: real_validators,
                    lost_reward_set: real_lost_rewards,
                    disabled_set: real_disabled_slots,
                    history_root: real_history_root,
                };

                if real_body.hash::<Blake2bHash>() != macro_block.header.body_root {
                    warn!("Rejecting block - Header body hash doesn't match real body hash");
                    return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
                }

                // Since we were not given a body, we return the body that we already calculated.
                return Ok(Some(real_body));
            }
        }

        Ok(None)
    }
}
