use std::cmp::Ordering;

use beserial::Serialize;
use nimiq_block::{
    Block, BlockBody, BlockError, BlockHeader, BlockType, ForkProof, MacroBlock, MacroBody,
    TendermintProof, ViewChange,
};
use nimiq_database::Transaction as DBtx;
use nimiq_hash::{Blake3Hash, Hash};
use nimiq_keys::PublicKey as SchnorrPublicKey;
use nimiq_primitives::policy;

use nimiq_transaction::Transaction;

use crate::blockchain_state::BlockchainState;
use crate::{AbstractBlockchain, Blockchain, PushError};

/// Implements methods to verify the validity of blocks.
impl Blockchain {
    /// Verifies the header of a block. This function is used when we are pushing a normal block
    /// into the chain. It cannot be used when syncing, since this performs more strict checks than
    /// the ones we make when syncing.
    /// This only performs checks that can be made BEFORE the state is updated with the block. All
    /// checks that require the updated state (ex: if an account has enough funds) are made on the
    /// verify_block_state method.
    // Note: This is an associated method because we need to use it on the nano-blockchain. There
    //       might be a better way to do this though.
    pub fn verify_block_header<B: AbstractBlockchain>(
        blockchain: &B,
        header: &BlockHeader,
        signing_key: &SchnorrPublicKey,
        txn_opt: Option<&DBtx>,
        check_seed: bool,
    ) -> Result<(), PushError> {
        // Check the version
        if header.version() != policy::VERSION {
            warn!(
                "Rejecting block {} - wrong version ({} != {})",
                header,
                header.version(),
                policy::VERSION
            );
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_info = blockchain
            .get_chain_info(header.parent_hash(), false, txn_opt)
            .ok_or(PushError::Orphan)?;

        // Check that the block is a valid successor of its predecessor.
        let next_block_type = blockchain.get_next_block_type(Some(prev_info.head.block_number()));
        if header.ty() != next_block_type {
            warn!(
                "Rejecting block {} - wrong block type ({:?} != {:?})",
                header,
                header.ty(),
                next_block_type,
            );
            return Err(PushError::InvalidSuccessor);
        }

        // Check the block number.
        let next_block_number = prev_info.head.block_number() + 1;
        if header.block_number() != next_block_number {
            warn!(
                "Rejecting block {} - wrong block number ({} != {})",
                header,
                header.block_number(),
                next_block_number,
            );
            return Err(PushError::InvalidSuccessor);
        }

        // Check that the current block timestamp is equal or greater than the timestamp of the
        // previous block.
        if header.timestamp() < prev_info.head.timestamp() {
            warn!(
                "Rejecting block {} - block timestamp precedes parent timestamp ({} < {})",
                header,
                header.timestamp(),
                prev_info.head.timestamp(),
            );
            return Err(PushError::InvalidSuccessor);
        }

        // Check that the current block timestamp less the node's current time is less than or equal
        // to the allowed maximum drift. Basically, we check that the block isn't from the future.
        // Both times are given in Unix time standard in millisecond precision.
        let timestamp_diff = header.timestamp().saturating_sub(blockchain.now());
        if timestamp_diff > policy::TIMESTAMP_MAX_DRIFT {
            warn!(
                "Rejecting block {} - block timestamp {} exceeds allowed maximum drift ({} > {})",
                header,
                header.timestamp(),
                timestamp_diff,
                policy::TIMESTAMP_MAX_DRIFT
            );
            return Err(PushError::InvalidBlock(BlockError::FromTheFuture));
        }

        // Check if the seed was signed by the intended producer.
        if check_seed {
            if let Err(e) = header.seed().verify(prev_info.head.seed(), signing_key) {
                warn!("Rejecting block {} - invalid seed ({:?})", header, e);
                return Err(PushError::InvalidBlock(BlockError::InvalidSeed));
            }
        }

        if header.ty() == BlockType::Macro {
            // Check if the parent election hash matches the current election head hash
            let parent_election_hash = header.parent_election_hash().unwrap();
            if parent_election_hash != &blockchain.election_head_hash() {
                warn!(
                    "Rejecting block {} - wrong parent election hash ({} != {})",
                    header,
                    parent_election_hash,
                    blockchain.election_head_hash()
                );
                return Err(PushError::InvalidSuccessor);
            }
        }

        Ok(())
    }

    /// Verifies the justification of a block.
    // Note: This is an associated method because we need to use it on the nano-blockchain. There
    //       might be a better way to do this though.
    pub fn verify_block_justification<B: AbstractBlockchain>(
        blockchain: &B,
        block: &Block,
        signing_key: &SchnorrPublicKey,
        txn_opt: Option<&DBtx>,
        check_signature: bool,
    ) -> Result<(), PushError> {
        match block {
            Block::Micro(micro_block) => {
                // Checks if the justification exists. If yes, unwrap it.
                let justification = micro_block
                    .justification
                    .as_ref()
                    .ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

                if check_signature {
                    // Verify the signature on the justification.
                    let hash = block.hash();
                    if !signing_key.verify(&justification.signature, hash.as_slice()) {
                        warn!(
                            "Rejecting block {} - invalid signature for slot owner {:?}",
                            block, signing_key
                        );
                        return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                    }
                }

                // Check if a view change occurred - if so, validate the proof
                let prev_info = blockchain
                    .get_chain_info(block.parent_hash(), false, txn_opt)
                    .unwrap();

                let next_view_number = prev_info.head.next_view_number();
                let view_number = block.view_number();

                if view_number < next_view_number {
                    warn!(
                        "Rejecting block {} - decreasing view number {} < {}",
                        block, view_number, next_view_number
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidViewNumber));
                } else if view_number == next_view_number
                    && justification.view_change_proof.is_some()
                {
                    warn!(
                        "Rejecting block {} - must not contain view change proof",
                        block
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
                } else if view_number > next_view_number
                    && justification.view_change_proof.is_none()
                {
                    warn!(
                        "Rejecting block {} - missing view change proof ({} > {})",
                        block, view_number, next_view_number
                    );
                    return Err(PushError::InvalidBlock(BlockError::NoViewChangeProof));
                } else if view_number > next_view_number
                    && justification.view_change_proof.is_some()
                {
                    let view_change = ViewChange {
                        block_number: block.block_number(),
                        new_view_number: block.view_number(),
                        vrf_entropy: prev_info.head.seed().entropy(),
                    };

                    if !justification
                        .view_change_proof
                        .as_ref()
                        .unwrap()
                        .verify(&view_change, &blockchain.current_validators().unwrap())
                    {
                        warn!("Rejecting block {} - bad view change proof", block);
                        return Err(PushError::InvalidBlock(BlockError::InvalidViewChangeProof));
                    }
                }
            }
            Block::Macro(macro_block) => {
                // Verify the Tendermint proof.
                if check_signature
                    && !TendermintProof::verify(
                        macro_block,
                        &blockchain.current_validators().unwrap(),
                    )
                {
                    warn!(
                        "Rejecting block {} - macro block with bad justification",
                        block
                    );
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
    pub fn verify_block_body(
        &self,
        header: &BlockHeader,
        body_opt: &Option<BlockBody>,
        txn_opt: Option<&DBtx>,
        verify_txns: bool,
    ) -> Result<(), PushError> {
        // Checks if the body exists. If yes, unwrap it.
        let body = body_opt
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        match body {
            BlockBody::Micro(body) => {
                // Check the size of the body.
                let body_size = body.serialized_size();
                if body_size > policy::MAX_SIZE_MICRO_BODY {
                    warn!(
                        "Rejecting block {} - body size exceeds maximum size ({} > {})",
                        header,
                        body_size,
                        policy::MAX_SIZE_MICRO_BODY
                    );
                    return Err(PushError::InvalidBlock(BlockError::SizeExceeded));
                }

                // Check the body root.
                let body_hash = body.hash::<Blake3Hash>();
                if *header.body_root() != body_hash {
                    warn!(
                        "Rejecting block {} - header body hash doesn't match real body hash ({} != {})",
                        header,
                        header.body_root(),
                        body_hash
                    );
                    return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
                }

                // Validate the fork proofs.
                let mut previous_proof: Option<&ForkProof> = None;

                for proof in &body.fork_proofs {
                    // Ensure proofs are ordered and unique.
                    if let Some(previous) = previous_proof {
                        match previous.cmp(proof) {
                            Ordering::Equal => {
                                return Err(PushError::InvalidBlock(
                                    BlockError::DuplicateForkProof,
                                ));
                            }
                            Ordering::Greater => {
                                return Err(PushError::InvalidBlock(
                                    BlockError::ForkProofsNotOrdered,
                                ));
                            }
                            _ => (),
                        }
                    }

                    // Check that the proof is within the reporting window.
                    if !proof.is_valid_at(header.block_number()) {
                        return Err(PushError::InvalidBlock(BlockError::InvalidForkProof));
                    }

                    // Get intended slot owner for that block.
                    if let Some(slot) = self.get_proposer_at(
                        proof.header1.block_number,
                        proof.header1.view_number,
                        proof.prev_vrf_seed.entropy(),
                        txn_opt,
                    ) {
                        // Verify fork proof.
                        if let Err(e) = proof.verify(&slot.validator.signing_key) {
                            warn!("Rejecting block {} - bad fork proof: {:?}", header, e);
                            return Err(PushError::InvalidBlock(BlockError::InvalidForkProof));
                        }

                        previous_proof = Some(proof);
                    } else {
                        warn!(
                            "Rejecting block {} - bad fork proof: Couldn't calculate slot owner",
                            header
                        );
                        return Err(PushError::InvalidBlock(BlockError::InvalidForkProof));
                    }
                }

                // Verify transactions.
                let mut previous_tx: Option<&Transaction> = None;

                for tx in &body.transactions {
                    // Ensure transactions are ordered and unique.
                    if let Some(previous) = previous_tx {
                        match previous.cmp(tx) {
                            Ordering::Greater => {
                                return Err(PushError::InvalidBlock(
                                    BlockError::TransactionsNotOrdered,
                                ));
                            }
                            Ordering::Equal => {
                                return Err(PushError::InvalidBlock(
                                    BlockError::DuplicateTransaction,
                                ));
                            }
                            _ => (),
                        }
                    }

                    // Check that the transaction is within its validity window.
                    if !tx.is_valid_at(header.block_number()) {
                        return Err(PushError::InvalidBlock(BlockError::ExpiredTransaction));
                    }

                    if verify_txns && !self.tx_verification_cache.is_known(&tx.hash()) {
                        // Check intrinsic transaction invariants.
                        if let Err(e) = tx.verify(self.network_id) {
                            return Err(PushError::InvalidBlock(BlockError::InvalidTransaction(e)));
                        }
                    }

                    previous_tx = Some(tx);
                }
            }
            BlockBody::Macro(body) => {
                // Check the body root.
                let body_hash = body.hash::<Blake3Hash>();
                if *header.body_root() != body_hash {
                    warn!(
                        "Rejecting block {} - header body hash doesn't match real body hash ({} != {})",
                        header,
                        header.body_root(),
                        body_hash
                    );
                    return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
                }

                // In case of an election block make sure it contains validators and pk_tree_root,
                // if it is not an election block make sure it doesn't contain either.
                let is_election = policy::is_election_block_at(header.block_number());

                if is_election != body.validators.is_some() {
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if is_election != body.pk_tree_root.is_some() {
                    return Err(PushError::InvalidBlock(BlockError::InvalidPkTreeRoot));
                }

                // If this is an election block, check if the pk_tree_root matches the validators.
                if is_election {
                    let pk_tree_root = MacroBlock::pk_tree_root(body.validators.as_ref().unwrap());
                    if pk_tree_root != *body.pk_tree_root.as_ref().unwrap() {
                        return Err(PushError::InvalidBlock(BlockError::InvalidPkTreeRoot));
                    }
                }
            }
        }

        Ok(())
    }

    /// Verifies a block against the blockchain state AFTER it gets updated with the block (ex: checking if
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
    pub fn verify_block_state(
        &self,
        state: &BlockchainState,
        block: &Block,
        txn_opt: Option<&DBtx>,
    ) -> Result<Option<MacroBody>, PushError> {
        let accounts = &state.accounts;

        // Verify accounts hash.
        let accounts_hash = accounts.get_root(txn_opt);

        if *block.state_root() != accounts_hash {
            warn!(
                "Rejecting block {} - header accounts hash doesn't match real accounts hash ({} != {})",
                block,
                block.state_root(),
                accounts_hash,
            );
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        // Verify the history root.
        let real_history_root = self
            .history_store
            .get_history_tree_root(block.epoch_number(), txn_opt)
            .ok_or_else(|| {
                error!(
                    "Rejecting block {} - failed to fetch history tree root for epoch {} from store",
                    block,
                    block.epoch_number(),
                );
                PushError::InvalidBlock(BlockError::InvalidHistoryRoot)
            })?;

        if *block.history_root() != real_history_root {
            warn!(
                "Rejecting block {} - history root doesn't match real history root ({} != {})",
                block,
                block.history_root(),
                real_history_root,
            );
            return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
        }

        // For macro blocks we have additional checks. We simply construct what the body should be
        // from our own state and then compare it with the body hash in the header.
        if let Block::Macro(macro_block) = block {
            // Get the lost rewards and disabled sets.
            let staking_contract = self.get_staking_contract();

            let real_lost_rewards = staking_contract.previous_lost_rewards();

            let real_disabled_slots = staking_contract.previous_disabled_slots();

            // Get the validators.
            let real_validators = if macro_block.is_election_block() {
                Some(self.next_validators(&macro_block.header.seed))
            } else {
                None
            };

            // Check the real values against the block.
            if let Some(body) = &macro_block.body {
                // If we were given a body, then check each value against the corresponding value in
                // the body.
                if real_lost_rewards != body.lost_reward_set {
                    warn!(
                        "Rejecting block {} - lost rewards set doesn't match real lost rewards set",
                        block
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_disabled_slots != body.disabled_set {
                    warn!(
                        "Rejecting block {} - disabled set doesn't match real disabled set",
                        block
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_validators != body.validators {
                    warn!(
                        "Rejecting block {} - validators don't match real validators",
                        block
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                // We don't need to check the nano_zkp_hash here since it was already checked in the
                // `verify_block_body` method.
            } else {
                // If we were not given a body, then we construct a body from our values and check
                // its hash against the block header.
                let real_pk_tree_root = real_validators.as_ref().map(MacroBlock::pk_tree_root);

                let real_body = MacroBody {
                    validators: real_validators,
                    pk_tree_root: real_pk_tree_root,
                    lost_reward_set: real_lost_rewards,
                    disabled_set: real_disabled_slots,
                };

                let real_body_hash = real_body.hash::<Blake3Hash>();
                if macro_block.header.body_root != real_body_hash {
                    warn!(
                        "Rejecting block {} - header body hash doesn't match real body hash ({} != {})",
                        block,
                        macro_block.header.body_root,
                        real_body_hash
                    );
                    return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
                }

                // Since we were not given a body, we return the body that we already calculated.
                return Ok(Some(real_body));
            }
        }

        Ok(None)
    }
}
