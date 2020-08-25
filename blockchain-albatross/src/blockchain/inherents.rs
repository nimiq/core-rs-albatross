use account::inherent::AccountInherentInteraction;
use account::{Inherent, InherentType};
use beserial::Serialize;
use block::MacroHeader;
use block::{ForkProof, ViewChanges};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use database::Transaction;
use genesis::NetworkInfo;
use keys::Address;
use primitives::coin::Coin;
use primitives::policy;
use primitives::slot::{SlashedSlot, SlotBand};
use vrf::{AliasMethod, VrfUseCase};

use crate::blockchain_state::BlockchainState;
use crate::reward::block_reward_for_batch;
use crate::Blockchain;

/// Implements methods that create inherents.
impl Blockchain {
    /// Given fork proofs and view changes, it returns the respective slash inherents. It expects
    /// verified fork proofs and view changes.
    pub fn create_slash_inherents(
        &self,
        fork_proofs: &[ForkProof],
        view_changes: &Option<ViewChanges>,
        txn_option: Option<&Transaction>,
    ) -> Vec<Inherent> {
        let mut inherents = vec![];

        for fork_proof in fork_proofs {
            inherents.push(self.inherent_from_fork_proof(fork_proof, txn_option));
        }

        if let Some(view_changes) = view_changes {
            inherents.append(&mut self.inherents_from_view_changes(view_changes, txn_option));
        }

        inherents
    }

    /// It creates a slash inherent from a fork proof. It expects a *verified* fork proof!
    pub fn inherent_from_fork_proof(
        &self,
        fork_proof: &ForkProof,
        txn_option: Option<&Transaction>,
    ) -> Inherent {
        // Get the address of the validator registry/staking contract.
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");

        // Get the slot owner and slot number for this block number and view number.
        let (producer, slot) = self.get_slot_owner_at(
            fork_proof.header1.block_number,
            fork_proof.header1.view_number,
            txn_option,
        );

        // Create the SlashedSlot struct.
        let slot = SlashedSlot {
            slot,
            validator_key: producer.public_key().clone(),
            event_block: fork_proof.header1.block_number,
        };

        // Create the corresponding slash inherent.
        Inherent {
            ty: InherentType::Slash,
            target: validator_registry.clone(),
            value: Coin::ZERO,
            data: slot.serialize_to_vec(),
        }
    }

    /// It creates a slash inherent(s) from a view change(s). It expects a *verified* view change!
    pub fn inherents_from_view_changes(
        &self,
        view_changes: &ViewChanges,
        txn_option: Option<&Transaction>,
    ) -> Vec<Inherent> {
        // Get the address of the validator registry/staking contract.
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");

        // Iterate over all the view changes.
        (view_changes.first_view_number..view_changes.last_view_number)
            .map(|view_number| {
                // Get the slot owner and slot number for this block number and view number.
                let (producer, slot) =
                    self.get_slot_owner_at(view_changes.block_number, view_number, txn_option);

                debug!("Slash inherent: view change: {}", producer.public_key());

                // Create the SlashedSlot struct.
                let slot = SlashedSlot {
                    slot,
                    validator_key: producer.public_key().clone(),
                    event_block: view_changes.block_number,
                };

                // Create the corresponding slash inherent.
                Inherent {
                    ty: InherentType::Slash,
                    target: validator_registry.clone(),
                    value: Coin::ZERO,
                    data: slot.serialize_to_vec(),
                }
            })
            .collect::<Vec<Inherent>>()
    }

    /// Creates the inherents to finalize a batch. The inherents are for reward distribution and
    /// updating the StakingContract.
    pub fn finalize_previous_batch(
        &self,
        state: &BlockchainState,
        macro_header: &MacroHeader,
    ) -> Vec<Inherent> {
        let prev_macro_info = &state.macro_info;

        let staking_contract = self.get_staking_contract();

        // Special case for first batch: Batch 0 is finalized by definition.
        if policy::batch_at(macro_header.block_number) - 1 == 0 {
            return vec![];
        }

        // Get validator slots
        // NOTE: Fields `current_slots` and `previous_slots` are expected to always be set.
        let validator_slots = if policy::first_batch_of_epoch(macro_header.block_number) {
            &state
            .previous_slots
            .as_ref()
            .expect("Slots for last batch are missing")
            .validator_slots
        } else {
            &state
            .current_slots
            .as_ref()
            .expect("Slots for last batch are missing")
            .validator_slots
        };

        // Calculate the slashed set. As conjunction of the two sets.
        let lost_rewards_set = staking_contract.previous_lost_rewards();
        let disabled_set = staking_contract.previous_disabled_slots();
        let slashed_set = lost_rewards_set & disabled_set;

        // Total reward for the previous batch
        let block_reward = block_reward_for_batch(
            macro_header,
            &prev_macro_info.head.unwrap_macro_ref().header,
            self.genesis_supply,
            self.genesis_timestamp,
        );

        let tx_fees = prev_macro_info.cum_tx_fees;

        let reward_pot: Coin = block_reward + tx_fees;

        // Distribute reward between all slots and calculate the remainder
        let slot_reward = reward_pot / policy::SLOTS as u64;
        let remainder = reward_pot % policy::SLOTS as u64;

        // The first slot number of the current validator
        let mut first_slot_number = 0;

        // Peekable iterator to collect slashed slots for validator
        let mut slashed_set_iter = slashed_set.iter().peekable();

        // All accepted inherents.
        let mut inherents = Vec::new();

        // Remember the number of eligible slots that a validator had (that was able to accept the inherent)
        let mut num_eligible_slots_for_accepted_inherent = Vec::new();

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

            // Create inherent for the reward
            let inherent = Inherent {
                ty: InherentType::Reward,
                target: validator_slot.reward_address().clone(),
                value: reward,
                data: vec![],
            };

            // Test whether account will accept inherent. If it can't then the reward will be
            // burned.
            let account = state.accounts.get(&inherent.target, None);

            if account
                .check_inherent(&inherent, macro_header.block_number)
                .is_err()
            {
                debug!(
                    "{} can't accept epoch reward {}",
                    inherent.target, inherent.value
                );
                burned_reward += reward;
            } else {
                num_eligible_slots_for_accepted_inherent.push(num_eligible_slots);
                inherents.push(inherent);
            }

            // Update first_slot_number for next iteration
            first_slot_number = last_slot_number;
        }

        // Check that number of accepted inherents is equal to length of the map that gives us the
        // corresponding number of slots for that staker (which should be equal to the number of
        // validators that will receive rewards).
        assert_eq!(
            inherents.len(),
            num_eligible_slots_for_accepted_inherent.len()
        );

        // Get RNG from last block's seed and build lookup table based on number of eligible slots.
        let mut rng = macro_header.seed.rng(VrfUseCase::RewardDistribution, 0);
        let lookup = AliasMethod::new(num_eligible_slots_for_accepted_inherent);

        // Randomly give remainder to one accepting slot. We don't bother to distribute it over all
        // accepting slots because the remainder is always at most SLOTS - 1 Lunas.
        let index = lookup.sample(&mut rng);
        inherents[index].value += remainder;

        // Create the inherent for the burned reward.
        let inherent = Inherent {
            ty: InherentType::Reward,
            target: Address::burn_address(),
            value: burned_reward,
            data: vec![],
        };

        inherents.push(inherent);

        // Push FinalizeBatch inherent to update StakingContract.
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");

        inherents.push(Inherent {
            ty: InherentType::FinalizeBatch,
            target: validator_registry.clone(),
            value: Coin::ZERO,
            data: Vec::new(),
        });

        inherents
    }

    /// Creates the inherent to finalize an epoch. The inherent is for updating the StakingContract.
    pub fn finalize_previous_epoch(&self) -> Inherent {
        // Get the address of the validator registry/staking contract.
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");

        // Create the FinalizeEpoch inherent.
        Inherent {
            ty: InherentType::FinalizeEpoch,
            target: validator_registry.clone(),
            value: Coin::ZERO,
            data: Vec::new(),
        }
    }
}
