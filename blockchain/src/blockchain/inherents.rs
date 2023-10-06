use nimiq_account::StakingContract;
use nimiq_block::{EquivocationProof, MacroBlock, MacroHeader, SkipBlockInfo};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_database as db;
use nimiq_keys::Address;
use nimiq_primitives::{
    account::AccountType,
    coin::Coin,
    policy::Policy,
    slots_allocation::{JailedValidator, PenalizedSlot},
};
use nimiq_transaction::{inherent::Inherent, reward::RewardTransaction};
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
    /// Given equivocation proofs and (or) a skip block, it returns the respective punishment inherents. It expects
    /// verified equivocation proofs and (or) skip block.
    pub fn create_punishment_inherents(
        &self,
        block_number: u32,
        equivocation_proofs: &[EquivocationProof],
        skip_block_info: Option<SkipBlockInfo>,
        txn_option: Option<&db::TransactionProxy>,
    ) -> Vec<Inherent> {
        let mut inherents = vec![];

        for equivocation_proof in equivocation_proofs {
            trace!(
                ?equivocation_proof,
                "Creating inherent from equivocation proof",
            );
            inherents.push(self.inherent_from_equivocation_proof(
                block_number,
                equivocation_proof,
                txn_option,
            ));
        }

        if let Some(skip_block_info) = skip_block_info {
            trace!("Creating inherent from skip block: {:?}", skip_block_info);
            inherents.push(self.inherent_from_skip_block_info(&skip_block_info, txn_option));
        }

        inherents
    }

    /// It creates a jail inherent from an equivocation proof. It expects a *verified* equivocation proof!
    pub fn inherent_from_equivocation_proof(
        &self,
        block_number: u32,
        equivocation_proof: &EquivocationProof,
        txn_option: Option<&db::TransactionProxy>,
    ) -> Inherent {
        // If the reporting block is in a new epoch, we check if the proposer is still a validator in this epoch
        // and retrieve its new slots.
        let new_epoch_slot_range = if Policy::epoch_at(block_number)
            > Policy::epoch_at(equivocation_proof.block_number())
        {
            self.current_validators()
                .expect("We need to have validators")
                .get_validator_by_address(equivocation_proof.validator_address())
                .map(|validator| validator.slots.clone())
        } else {
            None
        };

        let validators = self
            .get_validators_for_epoch(
                Policy::epoch_at(equivocation_proof.block_number()),
                txn_option,
            )
            .expect("Couldn't calculate validators");
        // `equivocation_proof.validator_address()` is checked in `EquivocationProof::verify`.
        let validator = validators
            .get_validator_by_address(equivocation_proof.validator_address())
            .expect("Validator must have been present");

        let jailed_validator = JailedValidator {
            slots: validator.slots.clone(),
            validator_address: equivocation_proof.validator_address().clone(),
            offense_event_block: equivocation_proof.block_number(),
        };

        // Create the corresponding jail inherent.
        Inherent::Jail {
            jailed_validator,
            new_epoch_slot_range,
        }
    }

    /// It creates a penalize inherent from a skip block. It expects a *verified* skip block!
    pub fn inherent_from_skip_block_info(
        &self,
        skip_block_info: &SkipBlockInfo,
        txn_option: Option<&db::TransactionProxy>,
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
            "Penalize inherent from skip block"
        );

        // Create the PenalizedSlot struct.
        let slot = PenalizedSlot {
            slot: proposer_slot.number,
            validator_address: proposer_slot.validator.address,
            offense_event_block: skip_block_info.block_number,
        };

        // Create the corresponding penalize inherent.
        Inherent::Penalize { slot }
    }

    /// Creates the inherents to finalize a batch. The inherents are for reward distribution and
    /// updating the StakingContract.
    pub fn finalize_previous_batch(&self, macro_block: &MacroBlock) -> Vec<Inherent> {
        // Special case for first batch: Batch 0 is finalized by definition.
        if Policy::batch_at(macro_block.block_number()) - 1 == 0 {
            return vec![];
        }

        // To get the inherents we either fetch the reward transactions from the macro body;
        // or we create the transactions when there is no macro body.
        let mut inherents: Vec<Inherent> = if let Some(body) = macro_block.body.as_ref() {
            body.transactions.iter().map(Inherent::from).collect()
        } else {
            self.create_reward_transactions(
                self.state(),
                &macro_block.header,
                &self.get_staking_contract(),
            )
            .iter()
            .map(Inherent::from)
            .collect()
        };

        // Push FinalizeBatch inherent to update StakingContract.
        inherents.push(Inherent::FinalizeBatch);

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
                .expect("Slots for current batch are missing")
        };

        // Calculate the slots that will receive rewards.
        // Rewards are for the previous batch (to give validators time to report misbehavior)
        let penalized_set = staking_contract
            .punished_slots
            .previous_batch_punished_slots();

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

        // Peekable iterator to collect penalized slots for validator
        let mut penalized_set_iter = penalized_set.iter().peekable();

        // All accepted inherents.
        let mut transactions = Vec::new();

        // Remember the number of eligible slots that a validator had (that was able to accept the inherent)
        let mut num_eligible_slots_for_accepted_tx = Vec::new();

        // Remember that the total amount of reward must be burned. The reward for a slot is burned
        // either because the slot was penalized or because the corresponding validator was unable to
        // accept the inherent.
        let mut burned_reward = Coin::ZERO;

        // Compute inherents
        for validator_slot in validator_slots.iter() {
            // The interval of slot numbers for the current slot band is
            // [first_slot_number, last_slot_number). So it actually doesn't include
            // `last_slot_number`.
            let last_slot_number = first_slot_number + validator_slot.num_slots();

            // Compute the number of punishments for this validator slot band.
            let mut num_eligible_slots = validator_slot.num_slots();
            let mut num_penalized_slots = 0;

            while let Some(next_penalized_slot) = penalized_set_iter.peek() {
                let next_penalized_slot = *next_penalized_slot as u16;
                assert!(next_penalized_slot >= first_slot_number);
                if next_penalized_slot < last_slot_number {
                    assert!(num_eligible_slots > 0);
                    penalized_set_iter.next();
                    num_eligible_slots -= 1;
                    num_penalized_slots += 1;
                } else {
                    break;
                }
            }

            // Compute reward from slot reward and number of eligible slots. Also update the burned
            // reward from the number of penalized slots.
            let reward = slot_reward
                .checked_mul(num_eligible_slots as u64)
                .expect("Overflow in reward");

            burned_reward += slot_reward
                .checked_mul(num_penalized_slots as u64)
                .expect("Overflow in reward");

            // Do not create reward transactions for zero rewards
            if !reward.is_zero() {
                // Create inherent for the reward.
                let staking_contract = self.get_staking_contract();
                let data_store = self.get_staking_contract_store();
                let txn = self.read_transaction();
                let validator = staking_contract
                    .get_validator(&data_store.read(&txn), &validator_slot.address)
                    .expect("Couldn't find validator in the accounts trie when paying rewards!");

                let tx = RewardTransaction {
                    validator_address: validator.address.clone(),
                    recipient: validator.reward_address.clone(),
                    value: reward,
                };

                // Test whether account will accept inherent. If it can't then the reward will be
                // burned.
                // TODO Improve this check: it assumes that only BasicAccounts can receive transactions.
                let account = state.accounts.get_complete(&tx.recipient, Some(&txn));
                if account.account_type() == AccountType::Basic {
                    num_eligible_slots_for_accepted_tx.push(num_eligible_slots);
                    transactions.push(tx);
                } else {
                    debug!(
                        target_address = %tx.recipient,
                        reward = %tx.value,
                        "Can't accept batch reward"
                    );
                    burned_reward += reward;
                }
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
                validator_address: Address::burn_address(),
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
        Inherent::FinalizeEpoch
    }
}
