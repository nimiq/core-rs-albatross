use std::collections::{btree_set::BTreeSet, BTreeMap};

use nimiq_collections::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots::{Validators, ValidatorsBuilder},
};
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};
pub use receipts::*;
use serde::{Deserialize, Serialize};
pub use staker::Staker;
pub use store::StakingContractStore;
#[cfg(feature = "interaction-traits")]
pub use store::StakingContractStoreWrite;
pub use validator::{Tombstone, Validator};

use crate::{
    account::staking_contract::store::{StakingContractStoreRead, StakingContractStoreReadOps},
    data_store_ops::{DataStoreIterOps, DataStoreReadOps},
};

mod receipts;
mod staker;
mod store;
#[cfg(feature = "interaction-traits")]
mod traits;
mod validator;

/// The struct representing the staking contract. The staking contract is a special contract that
/// handles most functions related to validators and staking.
/// The overall staking contract is a subtrie in the AccountsTrie that is composed of several
/// different account types. Each different account type is intended to store a different piece of
/// data concerning the staking contract. By having the path to each account you can navigate the
/// staking contract subtrie. The subtrie has the following format:
///
/// ```text
/// STAKING_CONTRACT_ADDRESS: StakingContract
///     |--> PREFIX_VALIDATOR || VALIDATOR_ADDRESS: Validator
///     |--> PREFIX_TOMBSTONE || VALIDATOR_ADDRESS: Tombstone
///     |
///     |--> PREFIX_STAKER || STAKER_ADDRESS: Staker
/// ```
///
/// So, for example, if you want to get the validator with a given address then you just fetch the
/// node with key STAKING_CONTRACT_ADDRESS||PATH_VALIDATORS_LIST||VALIDATOR_ADDRESS||PATH_VALIDATOR_MAIN
/// from the AccountsTrie (|| means concatenation).
/// At a high level, the Staking Contract subtrie contains:
///     - The Staking contract main. A struct that contains general information about the Staking contract.
///     - A list of Validators. Each of them is a subtrie containing the Validator struct, with all
///       the information relative to the Validator.
///     - A list of Stakers, with each Staker struct containing all information about a staker.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakingContract {
    // The total amount of coins staked (also includes validators deposits).
    pub balance: Coin,
    // The list of active validators addresses (i.e. are eligible to receive slots) and their
    // corresponding balances.
    pub active_validators: BTreeMap<Address, Coin>,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the current epoch.
    pub current_batch_lost_rewards: BitSet,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the previous batch.
    pub previous_batch_lost_rewards: BitSet,
    // The validator slots, searchable by the validator address, that are disabled (i.e. are no
    // longer eligible to produce blocks) currently.
    pub current_epoch_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
    // The validator slots, searchable by the validator address, that were disabled (i.e. are no
    // longer eligible to produce blocks) during the previous epoch.
    pub previous_epoch_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
}

impl StakingContract {
    /// Get a validator given its address, if it exists.
    pub fn get_validator<T: DataStoreReadOps>(
        &self,
        data_store: &T,
        address: &Address,
    ) -> Option<Validator> {
        StakingContractStoreRead::new(data_store).get_validator(address)
    }

    /// Get a staker given its address, if it exists.
    pub fn get_staker<T: DataStoreReadOps>(
        &self,
        data_store: &T,
        address: &Address,
    ) -> Option<Staker> {
        StakingContractStoreRead::new(data_store).get_staker(address)
    }

    /// Get a tombstone given its address, if it exists.
    pub fn get_tombstone<T: DataStoreReadOps>(
        &self,
        data_store: &T,
        address: &Address,
    ) -> Option<Tombstone> {
        StakingContractStoreRead::new(data_store).get_tombstone(address)
    }

    /// Get a list containing the addresses of all stakers that are delegating for a given validator.
    /// IMPORTANT: This is a very expensive operation, iterating over all existing stakers in the contract.
    pub fn get_stakers_for_validator<T: DataStoreReadOps + DataStoreIterOps>(
        &self,
        data_store: &T,
        address: &Address,
    ) -> Vec<Staker> {
        let read = StakingContractStoreRead::new(data_store);

        if let Some(validator) = read.get_validator(address) {
            let num_stakers = validator.num_stakers;
            return read
                .iter_stakers()
                .filter(|staker| staker.delegation.as_ref() == Some(address))
                .take(num_stakers as usize)
                .collect();
        }
        vec![]
    }

    /// Given a seed, it randomly distributes the validator slots across all validators. It is
    /// used to select the validators for the next epoch.
    pub fn select_validators<T: DataStoreReadOps>(
        &self,
        data_store: &T,
        seed: &VrfSeed,
    ) -> Validators {
        let mut validator_addresses = Vec::with_capacity(self.active_validators.len());
        let mut validator_stakes = Vec::with_capacity(self.active_validators.len());

        for (address, coin) in &self.active_validators {
            validator_addresses.push(address);
            validator_stakes.push(u64::from(*coin));
        }

        let mut rng = seed.rng(VrfUseCase::ValidatorSlotSelection);

        let lookup = AliasMethod::new(validator_stakes);

        let mut slots_builder = ValidatorsBuilder::default();

        for _ in 0..Policy::SLOTS {
            let index = lookup.sample(&mut rng);

            let chosen_validator = self
                .get_validator(data_store, validator_addresses[index])
                .expect("Couldn't find a validator that was in the active validators list!");

            slots_builder.push(
                chosen_validator.address,
                chosen_validator.voting_key,
                chosen_validator.signing_key,
            );
        }

        slots_builder.build()
    }

    /// Returns a BitSet of slots that lost its rewards in the previous batch.
    pub fn previous_batch_lost_rewards(&self) -> BitSet {
        self.previous_batch_lost_rewards.clone()
    }

    /// Returns a BitSet of slots that lost its rewards in the current batch.
    pub fn current_batch_lost_rewards(&self) -> BitSet {
        self.current_batch_lost_rewards.clone()
    }

    /// Returns a BitSet of slots that were marked as disabled in the previous epoch.
    pub fn previous_epoch_disabled_slots(&self) -> BitSet {
        let mut bitset = BitSet::new();
        for slots in self.previous_epoch_disabled_slots.values() {
            for &slot in slots {
                bitset.insert(slot as usize);
            }
        }
        bitset
    }

    /// Returns a BitSet of slots that were marked as disabled in the current epoch.
    pub fn current_epoch_disabled_slots(&self) -> BitSet {
        let mut bitset = BitSet::new();
        for slots in self.current_epoch_disabled_slots.values() {
            for &slot in slots {
                bitset.insert(slot as usize);
            }
        }
        bitset
    }
}
