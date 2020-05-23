use std::cmp::Ordering;
use std::collections::btree_set::BTreeSet;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::iter::FromIterator;
use std::sync::Arc;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use bls::CompressedPublicKey as BlsPublicKey;
use keys::Address;
use primitives::slot::{Slots, SlotsBuilder};
use primitives::{coin::Coin, policy};
use transaction::{SignatureProof, Transaction};
use vrf::{AliasMethod, VrfSeed, VrfUseCase};

use crate::AccountError;

pub use self::validator::*;

pub mod actions;
pub mod validator;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactiveStake {
    pub balance: Coin,
    pub retire_time: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct SlashReceipt {
    newly_slashed: bool,
}

#[derive(Debug)]
pub struct StakingContract {
    pub balance: Coin,
    // Validators
    pub active_validators_sorted: BTreeSet<Arc<Validator>>,
    pub active_validators_by_key: HashMap<BlsPublicKey, Arc<Validator>>,
    pub inactive_validators_by_key: BTreeMap<BlsPublicKey, InactiveValidator>,
    pub current_epoch_parking: HashSet<BlsPublicKey>,
    pub previous_epoch_parking: HashSet<BlsPublicKey>,
    // Stake
    pub inactive_stake_by_address: HashMap<Address, InactiveStake>,
}

impl StakingContract {
    pub fn get_validator(&self, validator_key: &BlsPublicKey) -> Option<&Arc<Validator>> {
        self.active_validators_by_key
            .get(validator_key)
            .or_else(|| {
                self.inactive_validators_by_key
                    .get(validator_key)
                    .map(|inactive_validator| &inactive_validator.validator)
            })
    }

    pub fn get_active_stake(
        &self,
        validator_key: &BlsPublicKey,
        staker_address: &Address,
    ) -> Option<Coin> {
        let validator = self.active_validators_by_key.get(validator_key)?;
        validator
            .active_stake_by_address
            .read()
            .get(staker_address)
            .cloned()
    }

    /// Allows to modify both active and inactive validators.
    /// It returns a validator entry, which subsumes active and inactive validators.
    /// It also allows for deferred error handling after re-adding the validator using `restore_validator`.
    pub fn remove_validator(&mut self, validator_key: &BlsPublicKey) -> Option<ValidatorEntry> {
        if let Some(validator) = self.active_validators_by_key.remove(&validator_key) {
            self.active_validators_sorted.remove(&validator);
            Some(ValidatorEntry::new_active_validator(validator))
        } else {
            //  The else case is needed to ensure the validator_key still exists.
            if let Some(validator) = self.inactive_validators_by_key.remove(&validator_key) {
                Some(ValidatorEntry::new_inactive_validator(validator))
            } else {
                None
            }
        }
    }

    /// Restores/saves a validator entry.
    /// If modifying the validator failed, it is restored and the error is returned.
    /// This makes it possible to remove the validator using `remove_validator`,
    /// try modifying it and restoring it before the error is returned.
    pub fn restore_validator(
        &mut self,
        validator_entry: ValidatorEntry,
    ) -> Result<(), AccountError> {
        match validator_entry {
            ValidatorEntry::Active(validator, error) => {
                // Update/restore validator.
                self.active_validators_sorted.insert(Arc::clone(&validator));
                self.active_validators_by_key
                    .insert(validator.validator_key.clone(), validator);
                error.map(Err).unwrap_or(Ok(()))
            }
            ValidatorEntry::Inactive(validator, error) => {
                // Update/restore validator.
                self.inactive_validators_by_key
                    .insert(validator.validator.validator_key.clone(), validator);
                error.map(Err).unwrap_or(Ok(()))
            }
        }
    }

    pub fn select_validators(&self, seed: &VrfSeed) -> Slots {
        // TODO: Depending on the circumstances and parameters, it might be more efficient to store active stake in an unsorted Vec.
        // Then, we would not need to create the Vec here. But then, removal of stake is a O(n) operation.
        // Assuming that validator selection happens less frequently than stake removal, the current implementation might be ok.
        let mut potential_validators = Vec::with_capacity(self.active_validators_sorted.len());
        let mut weights: Vec<u64> = Vec::with_capacity(self.active_validators_sorted.len());

        debug!("Select validators: num_slots = {}", policy::SLOTS);

        // NOTE: `active_stake_sorted` is sorted from highest to lowest stake. `LookupTable`
        // expects the reverse ordering.
        for validator in self.active_validators_sorted.iter() {
            potential_validators.push(Arc::clone(validator));
            weights.push(validator.balance.into());
        }

        let mut slots_builder = SlotsBuilder::default();
        let lookup = AliasMethod::new(weights);
        let mut rng = seed.rng(VrfUseCase::ValidatorSelection, 0);

        for _ in 0..policy::SLOTS {
            let index = lookup.sample(&mut rng);

            let active_validator = &potential_validators[index];

            slots_builder.push(
                active_validator.validator_key.clone(),
                &active_validator.reward_address,
            );
        }

        slots_builder.build()
    }

    fn get_self_signer(transaction: &Transaction) -> Result<Address, AccountError> {
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;
        Ok(signature_proof.compute_signer())
    }
}

impl Serialize for StakingContract {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.balance, writer)?;

        // Active validators first.
        size += Serialize::serialize(&(self.active_validators_sorted.len() as u32), writer)?;
        for active_validator in self.active_validators_sorted.iter() {
            size += Serialize::serialize(active_validator, writer)?;
        }

        // Inactive validators next.
        size += Serialize::serialize(&(self.inactive_validators_by_key.len() as u32), writer)?;
        for (_, inactive_validator) in self.inactive_validators_by_key.iter() {
            size += Serialize::serialize(inactive_validator, writer)?;
        }

        // Parking.
        size += SerializeWithLength::serialize::<u32, _>(&self.current_epoch_parking, writer)?;
        size += SerializeWithLength::serialize::<u32, _>(&self.previous_epoch_parking, writer)?;

        // Collect remaining inactive stakes.
        let mut inactive_stakes = Vec::new();
        for (staker_address, inactive_stake) in self.inactive_stake_by_address.iter() {
            inactive_stakes.push((staker_address, inactive_stake));
        }
        inactive_stakes.sort_by(|a, b| {
            a.0.cmp(b.0)
                .then_with(|| a.1.balance.cmp(&b.1.balance))
                .then_with(|| a.1.retire_time.cmp(&b.1.retire_time))
        });

        size += Serialize::serialize(&(inactive_stakes.len() as u32), writer)?;
        for (staker_address, inactive_stake) in inactive_stakes {
            size += Serialize::serialize(staker_address, writer)?;
            size += Serialize::serialize(inactive_stake, writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.balance);

        size += Serialize::serialized_size(&0u32);
        for active_validator in self.active_validators_sorted.iter() {
            size += Serialize::serialized_size(active_validator);
        }

        size += Serialize::serialized_size(&0u32);
        for (_, inactive_validator) in self.inactive_validators_by_key.iter() {
            size += Serialize::serialized_size(inactive_validator);
        }

        size += SerializeWithLength::serialized_size::<u32>(&self.current_epoch_parking);
        size += SerializeWithLength::serialized_size::<u32>(&self.previous_epoch_parking);

        size += Serialize::serialized_size(&0u32);
        for (staker_address, inactive_stake) in self.inactive_stake_by_address.iter() {
            size += Serialize::serialized_size(staker_address);
            size += Serialize::serialized_size(inactive_stake);
        }

        size
    }
}

impl Deserialize for StakingContract {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let balance = Deserialize::deserialize(reader)?;

        let mut active_validators_sorted = BTreeSet::new();
        let mut active_validators_by_key = HashMap::new();
        let mut inactive_validators_by_key = BTreeMap::new();
        let mut inactive_stake_by_address = HashMap::new();

        let num_active_validators: u32 = Deserialize::deserialize(reader)?;
        for _ in 0..num_active_validators {
            let active_validator: Validator = Deserialize::deserialize(reader)?;
            let active_validator = Arc::new(active_validator);

            active_validators_sorted.insert(Arc::clone(&active_validator));
            active_validators_by_key
                .insert(active_validator.validator_key.clone(), active_validator);
        }

        let num_inactive_validators: u32 = Deserialize::deserialize(reader)?;
        for _ in 0..num_inactive_validators {
            let inactive_validator: InactiveValidator = Deserialize::deserialize(reader)?;

            inactive_validators_by_key.insert(
                inactive_validator.validator.validator_key.clone(),
                inactive_validator,
            );
        }

        let current_epoch_parking: HashSet<BlsPublicKey> =
            DeserializeWithLength::deserialize::<u32, _>(reader)?;
        let previous_epoch_parking: HashSet<BlsPublicKey> =
            DeserializeWithLength::deserialize::<u32, _>(reader)?;

        let num_inactive_stakes: u32 = Deserialize::deserialize(reader)?;
        for _ in 0..num_inactive_stakes {
            let staker_address = Deserialize::deserialize(reader)?;
            let inactive_stake = Deserialize::deserialize(reader)?;
            inactive_stake_by_address.insert(staker_address, inactive_stake);
        }

        Ok(StakingContract {
            balance,
            active_validators_sorted,
            active_validators_by_key,
            inactive_validators_by_key,
            current_epoch_parking,
            previous_epoch_parking,
            inactive_stake_by_address,
        })
    }
}

// Not really useful traits for StakingContracts.
// FIXME Assume a single staking contract for now, i.e. all staking contracts are equal.
impl PartialEq for StakingContract {
    fn eq(&self, _other: &StakingContract) -> bool {
        true
    }
}

impl Eq for StakingContract {}

impl PartialOrd for StakingContract {
    fn partial_cmp(&self, other: &StakingContract) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StakingContract {
    fn cmp(&self, _other: &Self) -> Ordering {
        Ordering::Equal
    }
}

impl Default for StakingContract {
    fn default() -> Self {
        StakingContract {
            balance: Coin::ZERO,
            active_validators_sorted: BTreeSet::new(),
            active_validators_by_key: HashMap::new(),
            inactive_validators_by_key: Default::default(),
            current_epoch_parking: HashSet::new(),
            previous_epoch_parking: HashSet::new(),
            inactive_stake_by_address: HashMap::new(),
        }
    }
}

impl Clone for StakingContract {
    fn clone(&self) -> Self {
        let active_validators_by_key = HashMap::from_iter(
            self.active_validators_by_key
                .iter()
                .map(|(key, value)| (key.clone(), Arc::new(value.as_ref().clone()))),
        );
        let inactive_validators_by_key = BTreeMap::from_iter(
            self.inactive_validators_by_key
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        StakingContract {
            balance: self.balance,
            active_validators_sorted: BTreeSet::from_iter(
                active_validators_by_key.values().cloned(),
            ),
            active_validators_by_key,
            inactive_validators_by_key,
            current_epoch_parking: self.current_epoch_parking.clone(),
            previous_epoch_parking: self.previous_epoch_parking.clone(),
            inactive_stake_by_address: self.inactive_stake_by_address.clone(),
        }
    }
}
