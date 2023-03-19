use std::collections::btree_set::BTreeSet;
use std::collections::BTreeMap;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_collections::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots::{Validators, ValidatorsBuilder},
};
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};

use crate::{
    account::staking_contract::store::{StakingContractStoreRead, StakingContractStoreReadOps},
    data_store_ops::DataStoreReadOps,
};

pub use receipts::*;
pub use staker::Staker;
#[cfg(feature = "interaction-traits")]
pub use store::StakingContractStoreWrite;
pub use validator::{Tombstone, Validator};

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
/// STAKING_CONTRACT_ADDRESS
///     |--> PATH_CONTRACT_MAIN: Staking(StakingContract)
///     |
///     |--> PATH_VALIDATORS_LIST
///     |       |--> VALIDATOR_ADDRESS
///     |               |--> PATH_VALIDATOR_MAIN: StakingValidator(Validator)
///     |               |--> PATH_VALIDATOR_STAKERS_LIST
///     |                       |--> STAKER_ADDRESS: StakingValidatorsStaker(Address)
///     |
///     |--> PATH_STAKERS_LIST
///             |--> STAKER_ADDRESS: StakingStaker(Staker)
/// ```
///
/// So, for example, if you want to get the validator with a given address then you just fetch the
/// node with key STAKING_CONTRACT_ADDRESS||PATH_VALIDATORS_LIST||VALIDATOR_ADDRESS||PATH_VALIDATOR_MAIN
/// from the AccountsTrie (|| means concatenation).
/// At a high level, the Staking Contract subtrie contains:
///     - The Staking contract main. A struct that contains general information about the Staking contract.
///     - A list of Validators. Each of them is a subtrie containing the Validator struct, with all
///       the information relative to the Validator and a list of stakers that are validating for
///       this validator (we store only the staker address).
///     - A list of Stakers, with each Staker struct containing all information about a staker.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct StakingContract {
    // The total amount of coins staked (also includes validators deposits).
    pub balance: Coin,
    // The list of active validators addresses (i.e. are eligible to receive slots) and their
    // corresponding balances.
    pub active_validators: BTreeMap<Address, Coin>,
    // The validators that were parked during the current epoch.
    pub parked_set: BTreeSet<Address>,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the current batch.
    pub current_lost_rewards: BitSet,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the previous batch.
    pub previous_lost_rewards: BitSet,
    // The validator slots, searchable by the validator address, that are disabled (i.e. are no
    // longer eligible to produce blocks) currently.
    pub current_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
    // The validator slots, searchable by the validator address, that were disabled (i.e. are no
    // longer eligible to produce blocks) during the previous batch.
    pub previous_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
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
    pub fn get_stakers_for_validator<T: DataStoreReadOps>(
        &self,
        _data_store: &T,
        _address: &Address,
    ) -> Vec<Address> {
        todo!()
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
    pub fn previous_lost_rewards(&self) -> BitSet {
        self.previous_lost_rewards.clone()
    }

    /// Returns a BitSet of slots that lost its rewards in the current batch.
    pub fn current_lost_rewards(&self) -> BitSet {
        self.current_lost_rewards.clone()
    }

    /// Returns a BitSet of slots that were marked as disabled in the previous batch.
    pub fn previous_disabled_slots(&self) -> BitSet {
        let mut bitset = BitSet::new();
        for slots in self.previous_disabled_slots.values() {
            for &slot in slots {
                bitset.insert(slot as usize);
            }
        }
        bitset
    }

    /// Returns a BitSet of slots that were marked as disabled in the current batch.
    pub fn current_disabled_slots(&self) -> BitSet {
        let mut bitset = BitSet::new();
        for slots in self.current_disabled_slots.values() {
            for &slot in slots {
                bitset.insert(slot as usize);
            }
        }
        bitset
    }

    /// Returns a Vector with the addresses of all the currently parked Validators.
    pub fn parked_set(&self) -> Vec<Address> {
        self.parked_set.iter().cloned().collect()
    }
}

impl Serialize for StakingContract {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;

        size += Serialize::serialize(&self.balance, writer)?;

        size += SerializeWithLength::serialize::<u32, _>(&self.active_validators, writer)?;

        size += SerializeWithLength::serialize::<u32, _>(&self.parked_set, writer)?;

        size += Serialize::serialize(&self.current_lost_rewards, writer)?;
        size += Serialize::serialize(&self.previous_lost_rewards, writer)?;

        size += Serialize::serialize(&(self.current_disabled_slots.len() as u16), writer)?;
        for (key, slots) in self.current_disabled_slots.iter() {
            size += Serialize::serialize(key, writer)?;
            size += SerializeWithLength::serialize::<u16, _>(slots, writer)?;
        }
        size += Serialize::serialize(&(self.previous_disabled_slots.len() as u16), writer)?;
        for (key, slots) in self.previous_disabled_slots.iter() {
            size += Serialize::serialize(key, writer)?;
            size += SerializeWithLength::serialize::<u16, _>(slots, writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.balance);

        size += SerializeWithLength::serialized_size::<u32>(&self.active_validators);

        size += SerializeWithLength::serialized_size::<u32>(&self.parked_set);

        size += Serialize::serialized_size(&self.current_lost_rewards);
        size += Serialize::serialized_size(&self.previous_lost_rewards);

        size += Serialize::serialized_size(&(self.current_disabled_slots.len() as u16));
        for (key, slots) in self.current_disabled_slots.iter() {
            size += Serialize::serialized_size(key);
            size += SerializeWithLength::serialized_size::<u16>(slots);
        }
        size += Serialize::serialized_size(&(self.previous_disabled_slots.len() as u16));
        for (key, slots) in self.previous_disabled_slots.iter() {
            size += Serialize::serialized_size(key);
            size += SerializeWithLength::serialized_size::<u16>(slots);
        }

        size
    }
}

impl Deserialize for StakingContract {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let balance = Deserialize::deserialize(reader)?;

        let active_validators = DeserializeWithLength::deserialize::<u32, _>(reader)?;

        let parked_set = DeserializeWithLength::deserialize::<u32, _>(reader)?;

        let current_lost_rewards = Deserialize::deserialize(reader)?;
        let previous_lost_rewards = Deserialize::deserialize(reader)?;

        let num_current_disabled_slots: u16 = Deserialize::deserialize(reader)?;
        let mut current_disabled_slots = BTreeMap::new();
        for _ in 0..num_current_disabled_slots {
            let key: Address = Deserialize::deserialize(reader)?;
            let value = DeserializeWithLength::deserialize::<u16, _>(reader)?;
            current_disabled_slots.insert(key, value);
        }

        let num_previous_disabled_slots: u16 = Deserialize::deserialize(reader)?;
        let mut previous_disabled_slots = BTreeMap::new();
        for _ in 0..num_previous_disabled_slots {
            let key: Address = Deserialize::deserialize(reader)?;
            let value = DeserializeWithLength::deserialize::<u16, _>(reader)?;
            previous_disabled_slots.insert(key, value);
        }

        Ok(StakingContract {
            balance,
            active_validators,
            parked_set,
            current_lost_rewards,
            previous_lost_rewards,
            current_disabled_slots,
            previous_disabled_slots,
        })
    }
}
