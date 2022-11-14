use std::collections::btree_set::BTreeSet;
use std::collections::BTreeMap;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_collections::BitSet;
use nimiq_database::{Transaction as DBTransaction, WriteTransaction};
use nimiq_keys::Address;
use nimiq_primitives::slots::{Validators, ValidatorsBuilder};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};
pub use receipts::*;
pub use staker::Staker;
pub use validator::Validator;

use crate::{Account, AccountsTrie};

mod receipts;
mod staker;
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
    /// This is the byte path for the main struct in the staking contract.
    pub const PATH_CONTRACT_MAIN: u8 = 0;

    /// This is the byte path for the validators list in the staking contract.
    pub const PATH_VALIDATORS_LIST: u8 = 1;

    /// This is the byte path for the stakers list in the staking contract.
    pub const PATH_STAKERS_LIST: u8 = 2;

    /// This is the byte path for the main struct for a single validator (in the validators list).
    pub const PATH_VALIDATOR_MAIN: u8 = 0;

    /// This is the byte path for the stakers list for a single validator (in the validators list).
    pub const PATH_VALIDATOR_STAKERS_LIST: u8 = 1;

    /// Returns the key in the AccountsTrie for the Staking contract struct.
    pub fn get_key_staking_contract() -> KeyNibbles {
        let mut bytes = Vec::with_capacity(21);
        bytes.extend(Policy::STAKING_CONTRACT_ADDRESS.as_bytes());
        bytes.push(StakingContract::PATH_CONTRACT_MAIN);

        KeyNibbles::from(bytes.as_slice())
    }

    /// Returns the key in the AccountsTrie for a Validator struct with a given validator address.
    pub fn get_key_validator(validator_address: &Address) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(42);
        bytes.extend(Policy::STAKING_CONTRACT_ADDRESS.as_bytes());
        bytes.push(StakingContract::PATH_VALIDATORS_LIST);
        bytes.extend(validator_address.as_slice());
        bytes.push(StakingContract::PATH_VALIDATOR_MAIN);

        KeyNibbles::from(bytes.as_slice())
    }

    /// Returns the key in the AccountsTrie for the given staker validating for the given validator.
    pub fn get_key_validator_staker(
        validator_address: &Address,
        staker_address: &Address,
    ) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(62);
        bytes.extend(Policy::STAKING_CONTRACT_ADDRESS.as_bytes());
        bytes.push(StakingContract::PATH_VALIDATORS_LIST);
        bytes.extend(validator_address.as_slice());
        bytes.push(StakingContract::PATH_VALIDATOR_STAKERS_LIST);
        bytes.extend(staker_address.as_slice());

        KeyNibbles::from(bytes.as_slice())
    }

    /// Returns the key in the AccountsTrie for a Staker struct with a given staker address.
    pub fn get_key_staker(staker_address: &Address) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(41);
        bytes.extend(Policy::STAKING_CONTRACT_ADDRESS.as_bytes());
        bytes.push(StakingContract::PATH_STAKERS_LIST);
        bytes.extend(staker_address.as_bytes());

        KeyNibbles::from(bytes.as_slice())
    }

    /// Get the staking contract information.
    pub fn get_staking_contract(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
    ) -> StakingContract {
        let key = StakingContract::get_key_staking_contract();
        match accounts_tree
            .get::<Account>(db_txn, &key)
            .expect("temporary until accounts rewrite")
        {
            Some(Account::Staking(contract)) => contract,
            _ => {
                unreachable!()
            }
        }
    }

    /// Get a validator information given its address, if it exists.
    pub fn get_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
        validator_address: &Address,
    ) -> Option<Validator> {
        let key = StakingContract::get_key_validator(validator_address);

        match accounts_tree
            .get::<Account>(db_txn, &key)
            .expect("temporary until accounts rewrite")
        {
            Some(Account::StakingValidator(validator)) => Some(validator),
            None => None,
            _ => {
                unreachable!()
            }
        }
    }

    /// Get a list containing the addresses of all the stakers that delegating for a given validator.
    pub fn get_validator_stakers(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
        validator_address: &Address,
    ) -> Vec<Address> {
        let key = StakingContract::get_key_validator(validator_address);
        let validator = match accounts_tree
            .get::<Account>(db_txn, &key)
            .expect("temporary until accounts rewrite")
        {
            Some(Account::StakingValidator(validator)) => validator,
            _ => return vec![],
        };

        let num_stakers = validator.num_stakers as usize;

        let empty_staker_key =
            StakingContract::get_key_validator_staker(validator_address, &Address::from([0; 20]));

        let chunk = accounts_tree.get_chunk(db_txn, &empty_staker_key, num_stakers);

        let mut stakers = vec![];

        for account in chunk {
            match account {
                Account::StakingValidatorsStaker(address) => stakers.push(address),
                _ => {
                    unreachable!()
                }
            }
        }

        stakers
    }

    /// Get a staker information given its address, if it exists.
    pub fn get_staker(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
        staker_address: &Address,
    ) -> Option<Staker> {
        let key = StakingContract::get_key_staker(staker_address);
        match accounts_tree
            .get::<Account>(db_txn, &key)
            .expect("temporary until accounts rewrite")
        {
            Some(Account::StakingStaker(staker)) => Some(staker),
            None => None,
            _ => {
                unreachable!()
            }
        }
    }

    /// Creates a new Staking contract into the given accounts tree.
    pub fn create(accounts_tree: &AccountsTrie, db_txn: &mut WriteTransaction) {
        accounts_tree
            .put(
                db_txn,
                &StakingContract::get_key_staking_contract(),
                Account::Staking(StakingContract::default()),
            )
            .expect("temporary until accounts rewrite")
    }

    /// Given a seed, it randomly distributes the validator slots across all validators. It is
    /// used to select the validators for the next epoch.
    pub fn select_validators(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
        seed: &VrfSeed,
    ) -> Validators {
        let staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let mut validator_addresses = Vec::with_capacity(staking_contract.active_validators.len());
        let mut validator_stakes = Vec::with_capacity(staking_contract.active_validators.len());

        for (address, coin) in &staking_contract.active_validators {
            validator_addresses.push(address);
            validator_stakes.push(u64::from(*coin));
        }

        let mut rng = seed.rng(VrfUseCase::ValidatorSlotSelection);

        let lookup = AliasMethod::new(validator_stakes);

        let mut slots_builder = ValidatorsBuilder::default();

        for _ in 0..Policy::SLOTS {
            let index = lookup.sample(&mut rng);

            let chosen_validator =
                StakingContract::get_validator(accounts_tree, db_txn, validator_addresses[index]).expect("Couldn't find in the accounts tree a validator that was in the active validators list!");

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

    /// Checks if a given sender can pay the transaction.
    pub fn can_pay_tx(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
        tx_proof: OutgoingStakingTransactionProof,
        tx_value: Coin,
        block_height: u32,
    ) -> bool {
        match tx_proof {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                // Get the validator.
                let validator =
                    match StakingContract::get_validator(accounts_tree, db_txn, &validator_address)
                    {
                        Some(v) => v,
                        None => {
                            warn!(
                                "Cannot pay fees for transaction because validator doesn't exist."
                            );
                            return false;
                        }
                    };

                // If the fee is larger than the validator deposit then this won't work.
                if tx_value > validator.deposit {
                    warn!(
    "Cannot pay fees for transaction because fee is larger than validator deposit."
);
                    return false;
                }

                // Check that the validator has been inactive for long enough.
                match validator.inactivity_flag {
                    None => {
                        warn!("Cannot pay fees for transaction because validator is still active.");
                        return false;
                    }
                    Some(time) => {
                        if block_height
                            <= Policy::election_block_after(time) + Policy::blocks_per_batch()
                        {
                            warn!(
                    "Cannot pay fees for transaction because validator hasn't been inactive for long enough."
                );
                            return false;
                        }
                    }
                }
            }
            OutgoingStakingTransactionProof::Unstake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                // Get the staker.
                let staker =
                    match StakingContract::get_staker(accounts_tree, db_txn, &staker_address) {
                        Some(v) => v,
                        None => {
                            warn!("Cannot pay fees for transaction because staker doesn't exist.");
                            return false;
                        }
                    };

                // Check that the balance is enough to pay the fee.
                if tx_value > staker.balance {
                    warn!(
                    "Cannot pay fees for transaction because fee is larger than staker balance."
                );
                    return false;
                }
            }
        }

        true
    }

    /// Checks if a given creation transaction will succeed.
    pub fn can_create(
        accounts_tree: &AccountsTrie,
        db_txn: &DBTransaction,
        tx_data: IncomingStakingTransactionData,
    ) -> bool {
        match tx_data {
            IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                // If the validator already exists then the transaction would fail.
                if StakingContract::get_validator(accounts_tree, db_txn, &validator_address)
                    .is_some()
                {
                    warn!("Cannot create validator because validator already exists.");
                    return false;
                };
            }
            IncomingStakingTransactionData::CreateStaker { proof, delegation } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                // If the staker already exists then the transaction would fail.
                if StakingContract::get_staker(accounts_tree, db_txn, &staker_address).is_some() {
                    warn!("Cannot create staker because staker already exists.");
                    return false;
                };

                // Verify we have a valid delegation address (if present)
                if let Some(delegation) = delegation {
                    let key = StakingContract::get_key_validator(&delegation);
                    if accounts_tree
                        .get::<Account>(db_txn, &key)
                        .expect("temporary until accounts rewrite")
                        .is_none()
                    {
                        return false;
                    }
                }
            }
            _ => {}
        }

        true
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
