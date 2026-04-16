use std::collections::BTreeMap;

use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots_allocation::{Validators, ValidatorsBuilder},
};
use nimiq_vrf::{DiscreteDistribution, VrfSeed, VrfUseCase};
pub use receipts::*;
use serde::{Deserialize, Serialize};
pub use staker::Staker;
pub use store::StakingContractStore;
#[cfg(feature = "interaction-traits")]
pub use store::StakingContractStoreWrite;
pub use validator::{Tombstone, Validator};

#[cfg(feature = "interaction-traits")]
use crate::TransactionLog;
use crate::{
    account::staking_contract::{
        punished_slots::PunishedSlots,
        store::{StakingContractStoreRead, StakingContractStoreReadOps},
    },
    data_store_ops::{DataStoreIterOps, DataStoreReadOps},
};

pub mod punished_slots;
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
/// node with key STAKING_CONTRACT_ADDRESS||PREFIX_VALIDATOR||VALIDATOR_ADDRESS from the AccountsTrie
/// (|| means concatenation).
/// At a high level, the Staking Contract subtrie contains:
///     - The Staking contract main. A struct that contains general information about the
///       Staking contract (total balance, active validators and punished slots).
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
    // The punished slots for the current and previous batches.
    pub punished_slots: PunishedSlots,
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

    /// Get the list of all stakers that are delegating for the given validator.
    /// IMPORTANT: This is potentially a very expensive operation!
    pub fn get_stakers_for_validator<T: DataStoreReadOps + DataStoreIterOps>(
        &self,
        data_store: &T,
        address: &Address,
    ) -> Vec<Staker> {
        self.iter_stakers_for_validator(data_store, address)
            .collect()
    }

    /// Get an iterator over all stakers that are delegating for the given validator.
    pub fn iter_stakers_for_validator<'a, T: DataStoreReadOps + DataStoreIterOps + 'a>(
        &self,
        data_store: &'a T,
        address: &'a Address,
    ) -> impl Iterator<Item = Staker> + 'a {
        let read = StakingContractStoreRead::new(data_store);

        let num_stakers = read
            .get_validator(address)
            .map(|validator| validator.num_stakers)
            .unwrap_or(0);

        read.iter_stakers()
            .filter(|staker| staker.delegation.as_ref() == Some(address))
            .take(num_stakers as usize)
    }

    /// Get the list of all validators in the contract.
    /// IMPORTANT: This is potentially a very expensive operation!
    pub fn get_validators<T: DataStoreReadOps + DataStoreIterOps>(
        &self,
        data_store: &T,
    ) -> Vec<Validator> {
        self.iter_validators(data_store).collect()
    }

    /// Get an iterator over all validators in the contract.
    pub fn iter_validators<'a, T: DataStoreReadOps + DataStoreIterOps + 'a>(
        &self,
        data_store: &'a T,
    ) -> impl Iterator<Item = Validator> + 'a {
        StakingContractStoreRead::new(data_store).iter_validators()
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

        assert!(
            !validator_stakes.is_empty(),
            "Cannot select validators: no active validators exist"
        );
        let lookup = DiscreteDistribution::new(&validator_stakes);

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

    /// Returns the amount of active stake (i.e., total stake of active validators).
    pub fn get_active_stake(&self) -> Coin {
        self.active_validators.values().copied().sum()
    }

    /// Returns the total amount of coins that are supporting the upgrade and are active.
    /// The support is determined by a function over the signal data.
    /// IMPORTANT: This is a fairly expensive function, iterating over all validators.
    pub fn get_supporting_stake<
        T: DataStoreReadOps + DataStoreIterOps,
        F: Fn(Option<Blake2bHash>) -> bool,
    >(
        &self,
        data_store: &T,
        support_check: F,
    ) -> Coin {
        let staking_contract_read = StakingContractStoreRead::new(data_store);
        self.active_validators
            .iter()
            .filter_map(|(validator_address, stake)| {
                let validator_signal_data = staking_contract_read
                    .get_validator(validator_address)
                    .expect("Active Validators must be present in the Staking Contract.")
                    .signal_data;
                support_check(validator_signal_data).then_some(*stake)
            })
            .sum()
    }

    /// Returns the total amount of slots that are supporting the upgrade.
    /// The support is determined by a function over the signal data.
    pub fn get_supporting_slots<T: DataStoreReadOps, F: Fn(Option<Blake2bHash>) -> bool>(
        &self,
        data_store: &T,
        validators: &Validators,
        support_check: F,
    ) -> u16 {
        let staking_contract = StakingContractStoreRead::new(data_store);
        validators
            .iter()
            .map(|validator| {
                // If the validator cannot be found (e.g., because it has been deleted during the epoch),
                // it counts as non-supportive.
                let supports_upgrade = staking_contract
                    .get_validator(&validator.address)
                    .map(|validator| support_check(validator.signal_data))
                    .unwrap_or(false);

                if supports_upgrade {
                    validator.num_slots()
                } else {
                    0
                }
            })
            .sum()
    }

    /// Deactivates validators that did not support the upgrade.
    /// The support is determined by a function over the signal data.
    /// IMPORTANT: This is a fairly expensive function, iterating over all validators.
    #[cfg(feature = "interaction-traits")]
    pub(crate) fn deactivate_unsupporting_validators<F: Fn(Option<Blake2bHash>) -> bool>(
        &mut self,
        store: &mut StakingContractStoreWrite,
        support_check: F,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Retrieve unsupporting validators.
        let unsupporting_validators: Vec<_> = self
            .active_validators
            .iter()
            .filter_map(|(validator_address, _stake)| {
                let validator = store
                    .get_validator(validator_address)
                    .expect("Active Validators must be present in the Staking Contract.");
                if !support_check(validator.signal_data) {
                    Some((validator.address, Address::from(&validator.signing_key)))
                } else {
                    None
                }
            })
            .collect();

        // Deactivate those validators.
        for (validator_address, signer) in unsupporting_validators {
            // Since this function will deactivate the validator from the following election block,
            // we will pass `block_number - 1` as the block number. This ensures the validator is
            // inactive from this election block already.
            self.deactivate_validator(
                store,
                &validator_address,
                &signer,
                block_number - 1,
                tx_logger,
            )?;
        }

        Ok(())
    }
}
