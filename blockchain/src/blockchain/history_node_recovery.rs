use std::collections::VecDeque;

use nimiq_account::BlockLogger;
use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, ChainInfo, PushError};
use nimiq_database::traits::WriteTransaction;
use nimiq_primitives::policy::Policy;
use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use crate::Blockchain;

impl Blockchain {
    pub fn reset_to_latest_macro_block(
        this: RwLockUpgradableReadGuard<Self>,
        mut num_blocks: u32,
    ) -> Result<VecDeque<Block>, PushError> {
        // Store the blocks to be pushed later.
        let mut blocks_till_target = VecDeque::new();

        let head = this.head();
        let head_chain_info = this
            .get_chain_info(&head.hash(), true, None)
            .expect("Couldn't fetch chain info for the head of the chain");
        let target_block = head.block_number().saturating_sub(num_blocks);
        let is_reverting_across_macro_block =
            target_block < Policy::last_macro_block(head.block_number());

        let mut macro_info = None;
        if is_reverting_across_macro_block {
            log::info!("Reverting across a macro block requires a reset to the earliest macro block predecessor first.");
            let target_macro_block = this
                .get_block_at(Policy::last_macro_block(target_block), true, None)
                .unwrap()
                .unwrap_macro();
            macro_info = Some(
                this.get_chain_info(&target_macro_block.hash(), true, None)
                    .expect("Couldn't fetch chain info for the head of the chain"),
            );

            for _ in target_macro_block.block_number() + 1..target_block + 1 {
                blocks_till_target.push_front(
                    this.get_block_at(Policy::last_macro_block(target_block), true, None)
                        .unwrap(),
                );
            }

            num_blocks = num_blocks + (target_macro_block.block_number() - target_block);
        }

        log::error!("{}", num_blocks);

        // Delete all until target macro block
        let result = Self::revert_last_micro_blocks(&this, num_blocks)?;

        let mut this: RwLockWriteGuard<_> = RwLockUpgradableReadGuard::upgrade(this);
        if is_reverting_across_macro_block {
            // Assert that the state is consistent at this point.
            let macro_info = macro_info.unwrap();
            Self::revert_state(&mut this, &head_chain_info, &macro_info);

            let accounts_hash = this.state.accounts.get_root_hash_assert(None);
            assert_eq!(
                *macro_info.head.state_root(),
                accounts_hash,
                "Inconsistent state after reverting block {} - {:?}",
                *macro_info.head.state_root(),
                accounts_hash,
            );
        } else {
            log::error!(
                "head: {} Destination {}",
                head_chain_info.head.block_number(),
                result.head.block_number()
            );
            Self::revert_state(&mut this, &head_chain_info, &result);
        }

        Ok(blocks_till_target)
    }

    /// Reverts a given number of micro or skip blocks from the blockchain.
    pub fn revert_last_micro_blocks(
        this: &RwLockUpgradableReadGuard<Self>,
        num_blocks: u32,
    ) -> Result<ChainInfo, PushError> {
        debug!(
            num_blocks,
            "Need to revert micro blocks from the current epoch",
        );

        // Get the chain info for the head of the chain.
        let start_info = this
            .get_chain_info(&this.head_hash(), true, None)
            .expect("Couldn't fetch chain info for the head of the chain");
        let mut current_info = start_info.clone();
        let mut chain_infos = VecDeque::with_capacity((num_blocks - 1) as usize);
        for _ in 0..num_blocks {
            let chain_info = this
                .get_chain_info(current_info.head.parent_hash(), true, None)
                .expect("Failed to find main chain predecessor while reverting blocks");
            chain_infos.push_back(chain_info.clone());
            current_info = chain_info;
        }

        current_info = start_info;
        let mut reverted_macro_block = false;
        // Revert each block individually.
        while !chain_infos.is_empty() {
            // Get the chain info for the parent of the current head of the chain.
            let prev_info = chain_infos.pop_front().unwrap();
            {
                let mut a = this.write_transaction();
                // Revert the accounts tree. This also reverts the history store.
                this.revert_accounts(
                    &this.state.accounts,
                    &mut (&mut a).into(),
                    &current_info.head,
                    &mut BlockLogger::empty(),
                )?;
                a.commit();
            }

            // Check that the block reverted cleanly.
            // If we reverted a macro block the state will be inconsistent until we fix it.
            reverted_macro_block |= current_info.head.is_macro();
            if reverted_macro_block {
                let accounts_hash = this.state.accounts.get_root_hash_assert(None);
                assert_eq!(
                    *prev_info.head.state_root(),
                    accounts_hash,
                    "Inconsistent state after reverting block {} - {:?}",
                    &current_info.head,
                    &current_info.head,
                );
            }
            // Move on to the next block.
            current_info = prev_info;
        }

        log::error!("Destination {}", current_info.head.block_number());

        Ok(current_info)
    }

    fn revert_state(
        this: &mut RwLockWriteGuard<Self>,
        current_info: &ChainInfo,
        prev_info: &ChainInfo,
    ) {
        // Get the chain info for the target block.
        let target_macro_block = this
            .get_block_at(
                Policy::last_macro_block(prev_info.head.block_number()),
                true,
                None,
            )
            .unwrap()
            .unwrap_macro();
        let target_election_block = this
            .get_block_at(
                Policy::last_election_block(prev_info.head.block_number()),
                true,
                None,
            )
            .unwrap()
            .unwrap_macro();

        let target_block_hash = prev_info.head.hash();

        let target_chain_info = this
            .get_chain_info(&target_block_hash, true, None)
            .expect("Couldn't fetch chain info for the head of the chain");
        let macro_chain_info = this
            .get_chain_info(&target_macro_block.hash(), true, None)
            .expect("Couldn't fetch chain info for the head of the chain");

        this.state.main_chain = target_chain_info;
        this.state.head_hash = target_block_hash.clone();

        // Revert the slots.
        if current_info.head.is_macro() {
            log::trace!("Reverting the macro block related state");
            this.state.macro_info = macro_chain_info;
            this.state.macro_head_hash = target_macro_block.hash().clone();
        }
        // Revert the election head state.
        if current_info.head.is_election() {
            let target_previous_epoch_election = this
                .get_block_at(
                    Policy::election_block_before(target_election_block.block_number()),
                    true,
                    None,
                )
                .unwrap()
                .unwrap_macro();
            log::trace!("Reverting the election block related state");
            this.state.election_head_hash = target_election_block.hash();
            this.state.current_slots = target_macro_block.get_validators(); // ITODO, this is wrong if there are punishments on the epoch
            this.state.previous_slots = target_previous_epoch_election.get_validators(); // ITODO, this is wrong if there are punishments on the epoch
            this.state.election_head = target_election_block;
        }

        Self::revert_chain_store(this, current_info, prev_info);
    }

    pub fn revert_chain_store(
        this: &mut RwLockWriteGuard<Self>,
        current_info: &ChainInfo,
        prev_info: &ChainInfo,
    ) {
        // Get the chain info for the target block.
        let old_head_height = current_info.head.block_number();

        let mut txn = this.write_transaction();

        for height in prev_info.head.block_number() + 1..old_head_height + 1 {
            let hashes = this.chain_store.get_block_hashes_at(height, Some(&txn));
            for hash in hashes {
                log::trace!("Reverting chain_store #{}:{}", height, hash);
                this.chain_store.remove_chain_info(&mut txn, &hash, height);
            }
        }

        this.chain_store.set_head(&mut txn, &prev_info.head.hash());

        txn.commit();
    }
}
