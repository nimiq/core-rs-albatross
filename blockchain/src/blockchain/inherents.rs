use beserial::Serialize;
use nimiq_account::{Inherent, InherentType, StakingContract};
use nimiq_block::{ForkProof, MacroBlock, MacroHeader, SkipBlockInfo};
use nimiq_database as db;
use nimiq_keys::Address;
use nimiq_primitives::{
    account::AccountType, coin::Coin, key_nibbles::KeyNibbles, policy::Policy, slots::SlashedSlot,
};
use nimiq_transaction::reward::RewardTransaction;
use nimiq_vrf::{AliasMethod, VrfUseCase};

use crate::{blockchain_state::BlockchainState, reward::block_reward_for_batch, Blockchain};

/// Implements methods that create inherents.
impl Blockchain {
    pub fn create_macro_block_inherents(&self, macro_block: &MacroBlock) -> Vec<Inherent> {
        let mut inherents: Vec<Inherent> = vec![];

        // Every macro block is the end of a batch, so we need to finalize the batch.
        inherents.append(&mut self.finalize_previous_batch(macro_block));

        // If this block is an election block, we also need to finalize the epoch.
        if Policy::is_election_block_at(macro_block.block_number()) {
            // On election the previous epoch needs to be finalized.
            // We can rely on `state` here, since we cannot revert macro blocks.
            inherents.push(self.finalize_previous_epoch());
        }

        inherents
    }
    /// Given fork proofs and (or) a skip block, it returns the respective slash inherents. It expects
    /// verified fork proofs and (or) skip block.
    pub fn create_slash_inherents(
        &self,
        fork_proofs: &[ForkProof],
        skip_block_info: Option<SkipBlockInfo>,
        txn_option: Option<&db::Transaction>,
    ) -> Vec<Inherent> {
        let mut inherents = vec![];

        for fork_proof in fork_proofs {
            trace!("Creating inherent from fork proof: {:?}", fork_proof);
            inherents.push(self.inherent_from_fork_proof(fork_proof, txn_option));
        }

        if let Some(skip_block_info) = skip_block_info {
            trace!("Creating inherent from skip block: {:?}", skip_block_info);
            inherents.push(self.inherent_from_skip_block_info(&skip_block_info, txn_option));
        }

        inherents
    }

    /// It creates a slash inherent from a fork proof. It expects a *verified* fork proof!
    pub fn inherent_from_fork_proof(
        &self,
        fork_proof: &ForkProof,
        txn_option: Option<&db::Transaction>,
    ) -> Inherent {
        // Get the slot owner and slot number for this block number.
        let proposer_slot = self
            .get_proposer_at(
                fork_proof.header1.block_number,
                fork_proof.header1.block_number,
                fork_proof.prev_vrf_seed.entropy(),
                txn_option,
            )
            .expect("Couldn't calculate slot owner!");

        // Create the SlashedSlot struct.
        let slot = SlashedSlot {
            slot: proposer_slot.number,
            validator_address: proposer_slot.validator.address,
            event_block: fork_proof.header1.block_number,
        };

        // Create the corresponding slash inherent.
        Inherent {
            ty: InherentType::Slash,
            target: self.staking_contract_address(),
            value: Coin::ZERO,
            data: slot.serialize_to_vec(),
        }
    }

    /// It creates a slash inherent from a skip block. It expects a *verified* skip block!
    pub fn inherent_from_skip_block_info(
        &self,
        skip_block_info: &SkipBlockInfo,
        txn_option: Option<&db::Transaction>,
    ) -> Inherent {
        // Get the slot owner and slot number for this block number.
        let proposer_slot = self
            .get_proposer_at(
                skip_block_info.block_number,
                skip_block_info.block_number,
                skip_block_info.vrf_entropy.clone(),
                txn_option,
            )
            .expect("Couldn't calculate slot owner!");

        debug!(
            address = %proposer_slot.validator.address,
            "Slash inherent from skip block"
        );

        // Create the SlashedSlot struct.
        let slot = SlashedSlot {
            slot: proposer_slot.number,
            validator_address: proposer_slot.validator.address,
            event_block: skip_block_info.block_number,
        };

        // Create the corresponding slash inherent.
        Inherent {
            ty: InherentType::Slash,
            target: self.staking_contract_address(),
            value: Coin::ZERO,
            data: slot.serialize_to_vec(),
        }
    }

    /// Creates the inherents to finalize a batch. The inherents are for reward distribution and
    /// updating the StakingContract.
    pub fn finalize_previous_batch(&self, macro_block: &MacroBlock) -> Vec<Inherent> {
        // Special case for first batch: Batch 0 is finalized by definition.
        if Policy::batch_at(macro_block.block_number()) - 1 == 0 {
            return vec![];
        }

        // At this point we expect the body to be set and reward transactions to be present.
        let mut inherents: Vec<Inherent> = macro_block
            .body
            .as_ref()
            .expect("Macro body is missing")
            .transactions
            .iter()
            .map(Inherent::from)
            .collect();

        // Push FinalizeBatch inherent to update StakingContract.
        inherents.push(Inherent {
            ty: InherentType::FinalizeBatch,
            target: self.staking_contract_address(),
            value: Coin::ZERO,
            data: Vec::new(),
        });

        inherents
    }

    /// Creates the inherents to finalize a batch. The inherents are for reward distribution and
    /// updating the StakingContract.
    pub fn create_reward_transactions(
        &self,
        state: &BlockchainState,
        macro_header: &MacroHeader,
        staking_contract: &StakingContract,
    ) -> Vec<RewardTransaction> {
        let prev_macro_info = &state.macro_info;

        // Special case for first batch: Batch 0 is finalized by definition.
        if Policy::batch_at(macro_header.block_number) - 1 == 0 {
            return vec![];
        }

        // Get validator slots
        // NOTE: Fields `current_slots` and `previous_slots` are expected to always be set.
        let validator_slots = if Policy::first_batch_of_epoch(macro_header.block_number) {
            state
                .previous_slots
                .as_ref()
                .expect("Slots for last batch are missing")
        } else {
            state
                .current_slots
                .as_ref()
                .expect("Slots for last batch are missing")
        };

        // Calculate the slots that will receive rewards.
        // Rewards are for the previous batch (to give validators time to report misbehavior)
        // lost_rewards_set (clears on batch end) makes rewards being lost for at least one batch
        // disabled_set (clears on epoch end) makes rewards being lost further if validator doesn't unpark
        let lost_rewards_set = staking_contract.previous_lost_rewards();
        let disabled_set = staking_contract.previous_disabled_slots();
        let slashed_set = lost_rewards_set | disabled_set;

        // Total reward for the previous batch
        let block_reward = block_reward_for_batch(
            macro_header,
            &prev_macro_info.head.unwrap_macro_ref().header,
            self.genesis_supply,
            self.genesis_timestamp,
        );

        let tx_fees = prev_macro_info.cum_tx_fees;

        let reward_pot = block_reward + tx_fees;

        // Distribute reward between all slots and calculate the remainder
        let slot_reward = reward_pot / Policy::SLOTS as u64;
        let remainder = reward_pot % Policy::SLOTS as u64;

        // The first slot number of the current validator
        let mut first_slot_number = 0;

        // Peekable iterator to collect slashed slots for validator
        let mut slashed_set_iter = slashed_set.iter().peekable();

        // All accepted inherents.
        let mut transactions = Vec::new();

        // Remember the number of eligible slots that a validator had (that was able to accept the inherent)
        let mut num_eligible_slots_for_accepted_tx = Vec::new();

        // Remember that the total amount of reward must be burned. The reward for a slot is burned
        // either because the slot was slashed or because the corresponding validator was unable to
        // accept the inherent.
        let mut burned_reward = Coin::ZERO;

        // Compute inherents
        for validator_slot in validator_slots.iter() {
            // The interval of slot numbers for the current slot band is
            // [first_slot_number, last_slot_number). So it actually doesn't include
            // `last_slot_number`.
            let last_slot_number = first_slot_number + validator_slot.num_slots();

            // Compute the number of slashes for this validator slot band.
            let mut num_eligible_slots = validator_slot.num_slots();
            let mut num_slashed_slots = 0;

            while let Some(next_slashed_slot) = slashed_set_iter.peek() {
                let next_slashed_slot = *next_slashed_slot as u16;
                assert!(next_slashed_slot >= first_slot_number);
                if next_slashed_slot < last_slot_number {
                    assert!(num_eligible_slots > 0);
                    slashed_set_iter.next();
                    num_eligible_slots -= 1;
                    num_slashed_slots += 1;
                } else {
                    break;
                }
            }

            // Compute reward from slot reward and number of eligible slots. Also update the burned
            // reward from the number of slashed slots.
            let reward = slot_reward
                .checked_mul(num_eligible_slots as u64)
                .expect("Overflow in reward");

            burned_reward += slot_reward
                .checked_mul(num_slashed_slots as u64)
                .expect("Overflow in reward");

            // Create inherent for the reward.
            let validator = StakingContract::get_validator(
                &self.state().accounts.tree,
                &self.read_transaction(),
                &validator_slot.address,
            )
            .expect("Accounts trie must be complete.")
            .expect("Couldn't find validator in the accounts trie when paying rewards!");

            let tx = RewardTransaction {
                recipient: validator.reward_address.clone(),
                value: reward,
            };

            // Test whether account will accept inherent. If it can't then the reward will be
            // burned.
            let account = state
                .accounts
                .get(&KeyNibbles::from(&tx.recipient), None)
                .expect("Incomplete trie.");

            if account.is_none() || account.unwrap().account_type() == AccountType::Basic {
                num_eligible_slots_for_accepted_tx.push(num_eligible_slots);
                transactions.push(tx);
            } else {
                debug!(
                    target_address = %tx.recipient,
                    reward = %tx.value,
                    "Can't accept epoch reward"
                );
                burned_reward += reward;
            }

            // Update first_slot_number for next iteration
            first_slot_number = last_slot_number;
        }

        // Check that number of accepted inherents is equal to length of the map that gives us the
        // corresponding number of slots for that staker (which should be equal to the number of
        // validators that will receive rewards).
        assert_eq!(transactions.len(), num_eligible_slots_for_accepted_tx.len());

        // Get RNG from last block's seed and build lookup table based on number of eligible slots.
        let mut rng = macro_header.seed.rng(VrfUseCase::RewardDistribution);
        let lookup = AliasMethod::new(num_eligible_slots_for_accepted_tx);

        // Randomly give remainder to one accepting slot. We don't bother to distribute it over all
        // accepting slots because the remainder is always at most SLOTS - 1 Lunas.
        let index = lookup.sample(&mut rng);
        transactions[index].value += remainder;

        // Create the inherent for the burned reward.
        if burned_reward > Coin::ZERO {
            let tx = RewardTransaction {
                recipient: Address::burn_address(),
                value: burned_reward,
            };

            transactions.push(tx);
        }

        transactions
    }

    /// Creates the inherent to finalize an epoch. The inherent is for updating the StakingContract.
    pub fn finalize_previous_epoch(&self) -> Inherent {
        // Create the FinalizeEpoch inherent.
        Inherent {
            ty: InherentType::FinalizeEpoch,
            target: self.staking_contract_address(),
            value: Coin::ZERO,
            data: Vec::new(),
        }
    }
}
