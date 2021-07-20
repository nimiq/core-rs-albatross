use std::collections::btree_set::BTreeSet;
use std::collections::BTreeMap;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};

use nimiq_collections::BitSet;
use nimiq_database::Transaction as DBTransaction;
use nimiq_keys::Address;
use nimiq_primitives::slots::{Validators, ValidatorsBuilder};
use nimiq_primitives::{coin::Coin, policy};

use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};

use crate::{Account, AccountsTree};

pub use receipts::*;
pub use staker::Staker;
pub use validator::Validator;

mod receipts;
mod staker;
mod traits;
mod validator;

/// The struct representing the staking contract. The staking contract is a special contract that
/// handles many functions related to validators and staking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingContract {
    // The total amount of coins staked.
    pub balance: Coin,
    // A list of active validators IDs (i.e. are eligible to receive slots) and their corresponding
    // balances.
    pub active_validators: BTreeMap<Address, Coin>,
    // The validators that were parked during the current epoch.
    pub current_epoch_parking: BTreeSet<Address>,
    // The validators that were parked during the previous epoch.
    pub previous_epoch_parking: BTreeSet<Address>,
    // The validator slots that lost rewards (i.e. are no longer eligible to receive rewards) during
    // the current batch.
    pub current_lost_rewards: BitSet,
    // The validator slots that lost rewards (i.e. are no longer eligible to receive rewards) during
    // the previous batch.
    pub previous_lost_rewards: BitSet,
    // The validator slots, searchable by the validator ID, that are disabled (i.e. are no
    // longer eligible to produce blocks) currently.
    pub current_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
    // The validator slots, searchable by the validator ID, that were disabled (i.e. are no
    // longer eligible to produce blocks) at the end of the previous batch.
    pub previous_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
}

impl StakingContract {
    // This is the byte path for the main struct in the staking contract.
    pub const PATH_CONTRACT_MAIN: u8 = 0;

    // This is the byte path for the validators list in the staking contract.
    pub const PATH_VALIDATORS_LIST: u8 = 1;

    // This is the byte path for the stakers list in the staking contract.
    pub const PATH_STAKERS_LIST: u8 = 2;

    // This is the byte path for the main struct for a single validator (in the validators list).
    pub const PATH_VALIDATOR_MAIN: u8 = 0;

    // This is the byte path for the stakers list for a single validator (in the validators list).
    pub const PATH_VALIDATOR_STAKERS_LIST: u8 = 1;

    pub fn get_key_staking_contract() -> KeyNibbles {
        let mut bytes = Vec::with_capacity(21);
        bytes.extend(
            Address::from_user_friendly_address(policy::STAKING_CONTRACT_ADDRESS)
                .expect("Couldn't parse the staking contract address!")
                .as_bytes(),
        );
        bytes.push(StakingContract::PATH_CONTRACT_MAIN);

        KeyNibbles::from(bytes.as_slice())
    }

    pub fn get_key_validator(validator_address: &Address) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(42);
        bytes.extend(
            Address::from_user_friendly_address(policy::STAKING_CONTRACT_ADDRESS)
                .expect("Couldn't parse the staking contract address!")
                .as_bytes(),
        );
        bytes.push(StakingContract::PATH_VALIDATORS_LIST);
        bytes.extend(validator_address.as_slice());
        bytes.push(StakingContract::PATH_VALIDATOR_MAIN);

        KeyNibbles::from(bytes.as_slice())
    }

    pub fn get_key_validator_staker(
        validator_address: &Address,
        staker_address: &Address,
    ) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(62);
        bytes.extend(
            Address::from_user_friendly_address(policy::STAKING_CONTRACT_ADDRESS)
                .expect("Couldn't parse the staking contract address!")
                .as_bytes(),
        );
        bytes.push(StakingContract::PATH_VALIDATORS_LIST);
        bytes.extend(validator_address.as_slice());
        bytes.push(StakingContract::PATH_VALIDATOR_STAKERS_LIST);
        bytes.extend(staker_address.as_slice());

        KeyNibbles::from(bytes.as_slice())
    }

    pub fn get_key_staker(staker_address: &Address) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(41);
        bytes.extend(
            Address::from_user_friendly_address(policy::STAKING_CONTRACT_ADDRESS)
                .expect("Couldn't parse the staking contract address!")
                .as_bytes(),
        );
        bytes.push(StakingContract::PATH_STAKERS_LIST);
        bytes.extend(staker_address.as_bytes());

        KeyNibbles::from(bytes.as_slice())
    }

    /// Get the staking contract information.
    pub fn get_staking_contract(
        accounts_tree: &AccountsTree,
        db_txn: &DBTransaction,
    ) -> StakingContract {
        let key = StakingContract::get_key_staking_contract();

        trace!(
            "Trying to fetch staking contract at key {}.",
            key.to_string()
        );

        match accounts_tree.get(db_txn, &key) {
            Some(Account::Staking(contract)) => {
                return contract;
            }
            _ => {
                unreachable!()
            }
        }
    }

    /// Get a validator information given its id, if it exists.
    pub fn get_validator(
        accounts_tree: &AccountsTree,
        db_txn: &DBTransaction,
        validator_address: &Address,
    ) -> Option<Validator> {
        let key = StakingContract::get_key_validator(validator_address);

        trace!(
            "Trying to fetch validator with id {} at key {}.",
            validator_address.to_string(),
            key.to_string()
        );

        match accounts_tree.get(db_txn, &key) {
            Some(Account::StakingValidator(validator)) => {
                return Some(validator);
            }
            None => {
                return None;
            }
            _ => {
                unreachable!()
            }
        }
    }

    /// Get a staker information given its address, if it exists.
    pub fn get_staker(
        accounts_tree: &AccountsTree,
        db_txn: &DBTransaction,
        staker_address: &Address,
    ) -> Option<Staker> {
        let key = StakingContract::get_key_staker(staker_address);

        trace!(
            "Trying to fetch staker with address {} at key {}.",
            staker_address.to_string(),
            key.to_string()
        );

        match accounts_tree.get(db_txn, &key) {
            Some(Account::StakingStaker(staker)) => {
                return Some(staker);
            }
            None => {
                return None;
            }
            _ => {
                unreachable!()
            }
        }
    }

    /// Given a seed, it randomly distributes the validator slots across all validators. It can be
    /// used to select the validators for the next epoch.
    pub fn select_validators(
        &self,
        accounts_tree: &AccountsTree,
        db_txn: &DBTransaction,
        seed: &VrfSeed,
    ) -> Validators {
        let mut validator_addresses = Vec::with_capacity(self.active_validators.len());
        let mut validator_stakes = Vec::with_capacity(self.active_validators.len());

        debug!("Selecting validators: num_slots = {}", policy::SLOTS);

        for (id, coin) in self.active_validators.iter() {
            validator_addresses.push(id);
            validator_stakes.push(u64::from(coin.clone()));
        }

        let mut rng = seed.rng(VrfUseCase::ValidatorSelection, 0);

        let lookup = AliasMethod::new(validator_stakes);

        let mut slots_builder = ValidatorsBuilder::default();

        for _ in 0..policy::SLOTS {
            let index = lookup.sample(&mut rng);

            let chosen_validator =
                StakingContract::get_validator(accounts_tree, db_txn, validator_addresses[index]).expect("Couldn't find in the accounts tree a validator that was in the active validators list!");

            slots_builder.push(
                chosen_validator.address.clone(),
                chosen_validator.validator_key.clone(),
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
}

impl Serialize for StakingContract {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;

        size += Serialize::serialize(&self.balance, writer)?;

        size += SerializeWithLength::serialize::<u32, _>(&self.active_validators, writer)?;

        size += SerializeWithLength::serialize::<u32, _>(&self.current_epoch_parking, writer)?;
        size += SerializeWithLength::serialize::<u32, _>(&self.previous_epoch_parking, writer)?;

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

        size += SerializeWithLength::serialized_size::<u32>(&self.current_epoch_parking);
        size += SerializeWithLength::serialized_size::<u32>(&self.previous_epoch_parking);

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

        let current_epoch_parking = DeserializeWithLength::deserialize::<u32, _>(reader)?;
        let previous_epoch_parking = DeserializeWithLength::deserialize::<u32, _>(reader)?;

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
            current_epoch_parking,
            previous_epoch_parking,
            current_lost_rewards,
            previous_lost_rewards,
            current_disabled_slots,
            previous_disabled_slots,
        })
    }
}
