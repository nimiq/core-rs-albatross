use nimiq_block::{
    Block, BlockError, MacroBlock, MacroBody, MicroJustification, SkipBlockInfo, TendermintProof,
};
use nimiq_database::{ReadTransaction, Transaction as DBtx};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::slots::Validator;
use nimiq_primitives::{policy::Policy, slots::Validators};
use nimiq_vrf::VrfSeed;

use crate::blockchain_state::BlockchainState;
use crate::{AbstractBlockchain, BlockSuccessor, Blockchain, PushError};

/// Implements methods to verify the validity of blocks.
impl Blockchain {
    /// Verifies a block for the current blockchain state.
    /// This method does a full verification on the block except for the block state checks.
    /// See `verify_block_state` for these type of checks.
    // Note: This is an associated method because we need to use it on the light-blockchain. There
    //       might be a better way to do this though.
    pub fn verify_block_for_current_state<B: AbstractBlockchain>(
        blockchain: &B,
        read_txn: &ReadTransaction,
        block: &Block,
        trusted: bool,
    ) -> Result<(), PushError> {
        // Do block intrinsic checks
        block.verify(!trusted)?;

        // Check block for predecessor block
        let prev_info = blockchain
            .get_chain_info(block.parent_hash(), false, Some(read_txn))
            .ok_or(PushError::Orphan)?;
        let predecessor = prev_info.head;
        Self::verify_block_for_predecessor(
            block,
            &predecessor,
            BlockSuccessor::Subsequent(blockchain.election_head_hash()),
        )?;

        // Check block for slot and validators
        // Get the intended block proposer.
        let offset = if let Block::Macro(macro_block) = &block {
            macro_block.round()
        } else {
            // Skip and micro block offset is block number
            block.block_number()
        };

        // In trusted don't do slot related checks since they are mostly signature verifications and are slow.
        if !trusted {
            let (proposer_slot, _) = blockchain
                .get_slot_owner_at(block.block_number(), offset, Some(read_txn))
                .ok_or_else(|| {
                    warn!(%block, reason = "failed to determine block proposer", "Rejecting block");
                    PushError::Orphan
                })?;

            Self::verify_block_for_slot(
                block,
                predecessor.seed(),
                Some(&proposer_slot),
                &blockchain.current_validators().unwrap(),
            )?;
        }

        Ok(())
    }

    /// Verifies a block given its predecessor.
    /// This only performs verifications where the predecessor is needed.
    /// Check Block.verify() or verify_block_for_slot for further checks.
    pub fn verify_block_for_predecessor(
        block: &Block,
        predecessor: &Block,
        sequence_type: BlockSuccessor,
    ) -> Result<(), PushError> {
        let header = block.header();

        // Check that the current block timestamp is equal or greater than the timestamp of the
        // previous block.
        if header.timestamp() < predecessor.timestamp() {
            warn!(
                header = %header,
                obtained_timestamp = header.timestamp(),
                parent_timestamp   = predecessor.timestamp(),
                reason = "Block timestamp precedes predecessor timestamp",
                "Rejecting block"
            );
            return Err(PushError::InvalidSuccessor);
        }

        // Check that skip blocks has the expected timestamp and that the VRF seed is carried over
        if block.is_skip() {
            if header.timestamp() != predecessor.timestamp() + Policy::BLOCK_PRODUCER_TIMEOUT {
                warn!(
                    header = %header,
                    obtained_timestamp = header.timestamp(),
                    expected_timestamp   = predecessor.timestamp() + Policy::BLOCK_PRODUCER_TIMEOUT,
                    reason = "Unexpected timestamp for a skip block",
                    "Rejecting block"
                );
                return Err(PushError::InvalidBlock(
                    BlockError::InvalidSkipBlockTimestamp,
                ));
            }
            // In skip blocks the VRF seed must be carried over (because a new VRF seed requires a new leader)
            if header.seed() != predecessor.seed() {
                warn!(header = %header,
                    reason = "Invalid seed",
                    "Rejecting skip block");
                return Err(PushError::InvalidBlock(BlockError::InvalidSeed));
            }
        }

        match sequence_type {
            BlockSuccessor::Subsequent(ref exp_parent_election_hash) => {
                // Check that blocks are actually subsequent
                if *block.parent_hash() != predecessor.hash() {
                    warn!(
                        header = %header,
                        reason = "Wrong parent hash",
                        parent_hash = %block.parent_hash(),
                        expected_parent_hash = %predecessor.hash(),
                        "Rejecting block",
                    );
                    return Err(PushError::InvalidSuccessor);
                }

                // Check that the block is a valid successor of its predecessor.
                let next_block_type = Self::get_next_block_type(predecessor.block_number());
                if header.ty() != next_block_type {
                    warn!(
                        header = %header,
                        reason = "Wrong block type",
                        "Rejecting block obtained_type={:?} expected_type={:?}", header.ty(), next_block_type
                    );
                    return Err(PushError::InvalidSuccessor);
                }

                // Check the block number.
                if !block.is_immediate_successor_of(predecessor) {
                    warn!(
                        header = %header,
                        obtained_block_number = block.block_number(),
                        expected_block_number = predecessor.block_number() + 1,
                        reason = "Wrong block number",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidSuccessor);
                }

                // Check if the parent election hash matches the expected election head hash
                if block.is_macro() {
                    let parent_election_hash = header.parent_election_hash().unwrap();
                    if parent_election_hash != exp_parent_election_hash {
                        warn!(
                            header = %header,
                            parent_election_hash = %parent_election_hash,
                            expected_hash = %exp_parent_election_hash,
                            reason = "Wrong parent election hash",
                            "Rejecting block"
                        );
                        return Err(PushError::InvalidSuccessor);
                    }
                }
            }
            BlockSuccessor::Macro => {
                // Check that blocks are macro block successors
                if block.is_election()
                    && predecessor.is_election()
                    && !block.is_election_successor_of(predecessor)
                {
                    warn!(
                        header = %header,
                        predecessor = %predecessor,
                        reason = "Election blocks are not macro block successors",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidSuccessor);
                } else if !block.is_macro_successor_of(predecessor) {
                    warn!(
                        header = %header,
                        predecessor = %predecessor,
                        reason = "Blocks are not macro block successors",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidSuccessor);
                }
                // Check the block number is a non decreasing block number
                if header.block_number() <= predecessor.block_number() {
                    warn!(
                        header = %header,
                        obtained_block_number = header.block_number(),
                        checked_block_number = predecessor.block_number(),
                        reason = "Decreasing block number",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidSuccessor);
                }

                // The expected parent election hash depends on the predecessor:
                // - Predecessor is an election macro block: this block must have its hash as
                //   parent election hash
                // - Predecessor is a checkpoint macro block: this block must have the same
                //   parent election hash
                let predecessor_hash = predecessor.hash();
                let exp_parent_election_hash = if predecessor.is_election() {
                    &predecessor_hash
                } else {
                    predecessor.parent_election_hash().unwrap()
                };
                // Check that the blocks have the same parent election hash
                // Check if the parent election hash matches the current election head hash
                let parent_election_hash = block.parent_election_hash().unwrap();
                if parent_election_hash != exp_parent_election_hash {
                    warn!(
                        header = %header,
                        parent_election_hash = %parent_election_hash,
                        blockchain_election_hash = %predecessor.parent_election_hash().unwrap(),
                        reason = "Wrong parent election hash",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidSuccessor);
                }
            }
        }
        Ok(())
    }

    /// Verifies a block given the slot.
    /// This only performs verifications where the slot or the set of current validators are needed.
    /// Check Block.verify() or verify_block_for_predecessor for further checks.
    /// Notes: Passing `None` to `proposer` skips the VRF seed and the MicroJustification verification.
    pub fn verify_block_for_slot(
        block: &Block,
        prev_seed: &VrfSeed,
        proposer: Option<&Validator>,
        validators: &Validators,
    ) -> Result<(), PushError> {
        // Check if the seed was signed by the intended producer.
        if let Some(validator) = proposer {
            // Skip block seeds are carried from the previous block and this is checked in
            // `verify_block_for_predecessor`.
            if !block.is_skip() {
                if let Err(e) = block.seed().verify(prev_seed, &validator.signing_key) {
                    warn!(header = %block.header(),
                  reason = "Invalid seed",
                  "Rejecting block vrf_error={:?}", e);
                    return Err(PushError::InvalidBlock(BlockError::InvalidSeed));
                }
            }
        }

        // Verify the block justification
        match block {
            Block::Micro(micro_block) => {
                // Checks if the justification exists. If yes, unwrap it.
                let justification = micro_block
                    .justification
                    .as_ref()
                    .ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

                // Verify the signature on the justification.
                match justification {
                    MicroJustification::Micro(justification) => {
                        if let Some(validator) = proposer {
                            let hash = block.hash();
                            let signing_key = validator.signing_key;
                            if !signing_key.verify(justification, hash.as_slice()) {
                                warn!(
                                    %block,
                                    %signing_key,
                                    reason = "Invalid signature for slot owner",
                                    "Rejecting block"
                                );
                                return Err(PushError::InvalidBlock(
                                    BlockError::InvalidJustification,
                                ));
                            }
                        }
                    }
                    MicroJustification::Skip(justification) => {
                        let skip_block_info = SkipBlockInfo {
                            block_number: micro_block.header.block_number,
                            vrf_entropy: micro_block.header.seed.entropy(),
                        };

                        if !justification.verify(&skip_block_info, validators) {
                            warn!(
                                    %block,
                                    reason = "Bad skip block proof",
                                    "Rejecting block");
                            return Err(PushError::InvalidBlock(BlockError::InvalidSkipBlockProof));
                        }
                    }
                }
            }
            Block::Macro(macro_block) => {
                // Verify the Tendermint proof.
                if !TendermintProof::verify(macro_block, validators) {
                    warn!(
                        %block,
                        reason = "Macro block with bad justification",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
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
                %block,
                header_root = %block.state_root(),
                accounts_root = %accounts_hash,
                reason = "Header accounts hash doesn't match real accounts hash",
                "Rejecting block"
            );
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        // Verify the history root.
        let real_history_root = self
            .history_store
            .get_history_tree_root(block.epoch_number(), txn_opt)
            .ok_or_else(|| {
                error!(
                    %block,
                    epoch_number = block.epoch_number(),
                    reason = "failed to fetch history tree root for epoch from store",
                    "Rejecting block"
                );
                PushError::InvalidBlock(BlockError::InvalidHistoryRoot)
            })?;

        if *block.history_root() != real_history_root {
            warn!(
                %block,
                block_root = %block.history_root(),
                history_root = %real_history_root,
                reason = "History root doesn't match real history root",
                "Rejecting block"
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
                        %block,
                        reason = "lost rewards set doesn't match real lost rewards set",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_disabled_slots != body.disabled_set {
                    warn!(
                        %block,
                        reason = "Disabled set doesn't match real disabled set",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_validators != body.validators {
                    warn!(
                        %block,
                        reason = "Validators don't match real validators",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                // We don't need to check the nano_zkp_hash here since it was already checked in the
                // `verify_block_body` method.
            } else {
                // If we were not given a body, then we construct a body from our values and check
                // its hash against the block header.
                let real_pk_tree_root = real_validators
                    .as_ref()
                    .and_then(|validators| MacroBlock::pk_tree_root(validators).ok());

                let real_body = MacroBody {
                    validators: real_validators,
                    pk_tree_root: real_pk_tree_root,
                    lost_reward_set: real_lost_rewards,
                    disabled_set: real_disabled_slots,
                };

                let real_body_hash = real_body.hash::<Blake2bHash>();
                if macro_block.header.body_root != real_body_hash {
                    warn!(
                        %block,
                        header_root = %macro_block.header.body_root,
                        body_hash   = %real_body_hash,
                        reason = "Header body hash doesn't match real body hash",
                        "Rejecting block"
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
