use std::cmp::Ordering;
use std::collections::btree_set::BTreeSet;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::iter::FromIterator;
use std::sync::Arc;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_bls::CompressedSignature as BlsSignature;
use nimiq_collections::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::account::ValidatorId;
use nimiq_primitives::slots::{Validators, ValidatorsBuilder};
use nimiq_primitives::{coin::Coin, policy};
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};

use crate::AccountError;

pub use self::validator::*;

pub mod actions;
pub mod validator;

/// Struct represent an inactive staker. An inactive staker is a staker that got its stake not
/// eligible for slot selection. In other words, this staker can no longer receive slots.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactiveStake {
    // The balance of the stake.
    pub balance: Coin,
    // The block number when the stake became inactive.
    pub retire_time: u32,
}

/// A receipt for slash inherents. It shows whether a given slot or validator was newly disabled,
/// lost rewards or parked by a specific slash inherent. This is necessary to be able to revert
/// slash inherents.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct SlashReceipt {
    newly_parked: bool,
    newly_disabled: bool,
    newly_lost_rewards: bool,
}

/// The struct representing the staking contract. The staking contract is a special contract that
/// handles many functions related to validators and staking.
#[derive(Debug)]
pub struct StakingContract {
    // The total amount of coins staked.
    pub balance: Coin,
    // The active validators (i.e. are eligible to receive slots) sorted by public key.
    pub active_validators_sorted: BTreeSet<Arc<Validator>>,
    // A hashmap of the active validators, indexed by the validator id
    pub active_validators_by_id: HashMap<ValidatorId, Arc<Validator>>,
    // A hashmap of the inactive stakers (i.e. are NOT eligible to receive slots), indexed by the
    // staker address.
    pub inactive_stake_by_address: HashMap<Address, InactiveStake>,
    // A tree of the inactive validators , searchable by the validator id.
    pub inactive_validators_by_id: BTreeMap<ValidatorId, InactiveValidator>,
    // The validators that were parked during the current epoch.
    pub current_epoch_parking: HashSet<ValidatorId>,
    // The validators that were parked during the previous epoch.
    pub previous_epoch_parking: HashSet<ValidatorId>,
    // The validator slots that lost rewards (i.e. are no longer eligible to receive rewards) during
    // the current batch.
    pub current_lost_rewards: BitSet,
    // The validator slots that lost rewards (i.e. are no longer eligible to receive rewards) during
    // the previous batch.
    pub previous_lost_rewards: BitSet,
    // The validator slots, searchable by the validator public key, that are disabled (i.e. are no
    // longer eligible to produce blocks) currently.
    pub current_disabled_slots: BTreeMap<ValidatorId, BTreeSet<u16>>,
    // The validator slots, searchable by the validator public key, that were disabled (i.e. are no
    // longer eligible to produce blocks) at the end of the previous batch.
    pub previous_disabled_slots: BTreeMap<ValidatorId, BTreeSet<u16>>,
}

impl StakingContract {
    /// Get a validator information given its id.
    pub fn get_validator(&self, validator_id: &ValidatorId) -> Option<&Arc<Validator>> {
        self.active_validators_by_id.get(validator_id).or_else(|| {
            self.inactive_validators_by_id
                .get(validator_id)
                .map(|inactive_validator| &inactive_validator.validator)
        })
    }

    /// Get the amount staked by a staker, given the public key of the corresponding validator and
    /// the address of the staker. Returns None if the active validator or the staker does not exist.
    pub fn get_active_stake(
        &self,
        validator_id: &ValidatorId,
        staker_address: &Address,
    ) -> Option<Coin> {
        let validator = self.active_validators_by_id.get(validator_id)?;

        validator
            .active_stake_by_address
            .read()
            .get(staker_address)
            .cloned()
    }

    /// Allows to modify both active and inactive validators.
    /// It returns a validator entry, which subsumes active and inactive validators.
    /// It also allows for deferred error handling after re-adding the validator using `restore_validator`.
    pub fn remove_validator(&mut self, validator_id: &ValidatorId) -> Option<ValidatorEntry> {
        if let Some(validator) = self.active_validators_by_id.remove(validator_id) {
            self.active_validators_sorted.remove(&validator);
            Some(ValidatorEntry::new_active_validator(validator))
        } else {
            //  The else case is needed to ensure the validator_key still exists.
            self.inactive_validators_by_id
                .remove(validator_id)
                .map(ValidatorEntry::new_inactive_validator)
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
                self.active_validators_by_id
                    .insert(validator.id.clone(), validator);
                error.map(Err).unwrap_or(Ok(()))
            }
            ValidatorEntry::Inactive(validator, error) => {
                // Update/restore validator.
                self.inactive_validators_by_id
                    .insert(validator.validator.id.clone(), validator);
                error.map(Err).unwrap_or(Ok(()))
            }
        }
    }

    /// Given a seed, it randomly distributes the validator slots across all validators. It can be
    /// used to select the validators for the next epoch.
    pub fn select_validators(&self, seed: &VrfSeed) -> Validators {
        // TODO: Depending on the circumstances and parameters, it might be more efficient to store
        // active stake in an unsorted Vec.
        // Then, we would not need to create the Vec here. But then, removal of stake is a O(n) operation.
        // Assuming that validator selection happens less frequently than stake removal, the current
        // implementation might be ok.
        let mut potential_validators = Vec::with_capacity(self.active_validators_sorted.len());
        let mut weights: Vec<u64> = Vec::with_capacity(self.active_validators_sorted.len());

        debug!("Select validators: num_slots = {}", policy::SLOTS);

        // NOTE: `active_validators_sorted` is sorted from highest to lowest stake. `LookupTable`
        // expects the reverse ordering.
        for validator in self.active_validators_sorted.iter() {
            potential_validators.push(Arc::clone(validator));
            weights.push(validator.balance.into());
        }

        let mut slots_builder = ValidatorsBuilder::default();
        let lookup = AliasMethod::new(weights);
        let mut rng = seed.rng(VrfUseCase::ValidatorSelection, 0);

        for _ in 0..policy::SLOTS {
            let index = lookup.sample(&mut rng);

            let active_validator = &potential_validators[index];

            slots_builder.push(
                active_validator.id.clone(),
                active_validator.validator_key.clone(),
            );
        }

        slots_builder.build()
    }

    /// Get the signature from a transaction.
    fn get_self_signer(transaction: &Transaction) -> Result<Address, AccountError> {
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        Ok(signature_proof.compute_signer())
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

    fn verify_signature_incoming(
        &self,
        transaction: &Transaction,
        validator_id: &ValidatorId,
        signature: &BlsSignature,
    ) -> Result<(), AccountError> {
        let validator = self
            .get_validator(validator_id)
            .ok_or(AccountError::InvalidForRecipient)?;
        let key = validator
            .validator_key
            .uncompress()
            .map_err(|_| AccountError::InvalidForRecipient)?;
        let sig = signature
            .uncompress()
            .map_err(|_| AccountError::InvalidSignature)?;

        // On incoming transactions, we need to reset the signature first.
        let mut tx_without_sig = transaction.clone();
        tx_without_sig.data = IncomingStakingTransactionData::set_validator_signature_on_data(
            &tx_without_sig.data,
            BlsSignature::default(),
        )?;
        let tx = tx_without_sig.serialize_content();

        if !key.verify(&tx, &sig) {
            warn!("Invalid signature");

            return Err(AccountError::InvalidSignature);
        }
        Ok(())
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
        size += Serialize::serialize(&(self.inactive_validators_by_id.len() as u32), writer)?;
        for (_, inactive_validator) in self.inactive_validators_by_id.iter() {
            size += Serialize::serialize(inactive_validator, writer)?;
        }

        // Parking.
        size += SerializeWithLength::serialize::<u32, _>(&self.current_epoch_parking, writer)?;
        size += SerializeWithLength::serialize::<u32, _>(&self.previous_epoch_parking, writer)?;

        // Lost rewards.
        size += Serialize::serialize(&self.current_lost_rewards, writer)?;
        size += Serialize::serialize(&self.previous_lost_rewards, writer)?;

        // Disabled slots.
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
        for (_, inactive_validator) in self.inactive_validators_by_id.iter() {
            size += Serialize::serialized_size(inactive_validator);
        }

        size += SerializeWithLength::serialized_size::<u32>(&self.current_epoch_parking);
        size += SerializeWithLength::serialized_size::<u32>(&self.previous_epoch_parking);

        // Lost rewards.
        size += Serialize::serialized_size(&self.current_lost_rewards);
        size += Serialize::serialized_size(&self.previous_lost_rewards);

        // Disabled slots.
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
        let mut active_validators_by_id = HashMap::new();
        let mut inactive_validators_by_id = BTreeMap::new();
        let mut inactive_stake_by_address = HashMap::new();

        let num_active_validators: u32 = Deserialize::deserialize(reader)?;
        for _ in 0..num_active_validators {
            let active_validator: Validator = Deserialize::deserialize(reader)?;
            let active_validator = Arc::new(active_validator);

            active_validators_sorted.insert(Arc::clone(&active_validator));
            active_validators_by_id.insert(active_validator.id.clone(), active_validator);
        }

        let num_inactive_validators: u32 = Deserialize::deserialize(reader)?;
        for _ in 0..num_inactive_validators {
            let inactive_validator: InactiveValidator = Deserialize::deserialize(reader)?;

            inactive_validators_by_id
                .insert(inactive_validator.validator.id.clone(), inactive_validator);
        }

        let current_epoch_parking: HashSet<ValidatorId> =
            DeserializeWithLength::deserialize::<u32, _>(reader)?;
        let previous_epoch_parking: HashSet<ValidatorId> =
            DeserializeWithLength::deserialize::<u32, _>(reader)?;

        // Lost rewards.
        let current_lost_rewards: BitSet = Deserialize::deserialize(reader)?;
        let previous_lost_rewards: BitSet = Deserialize::deserialize(reader)?;

        // Disabled slots.
        let num_current_disabled_slots: u16 = Deserialize::deserialize(reader)?;
        let mut current_disabled_slots = BTreeMap::new();
        for _ in 0..num_current_disabled_slots {
            let key: ValidatorId = Deserialize::deserialize(reader)?;
            let value = DeserializeWithLength::deserialize::<u16, _>(reader)?;
            current_disabled_slots.insert(key, value);
        }
        let num_previous_disabled_slots: u16 = Deserialize::deserialize(reader)?;
        let mut previous_disabled_slots = BTreeMap::new();
        for _ in 0..num_previous_disabled_slots {
            let key: ValidatorId = Deserialize::deserialize(reader)?;
            let value = DeserializeWithLength::deserialize::<u16, _>(reader)?;
            previous_disabled_slots.insert(key, value);
        }

        let num_inactive_stakes: u32 = Deserialize::deserialize(reader)?;
        for _ in 0..num_inactive_stakes {
            let staker_address = Deserialize::deserialize(reader)?;
            let inactive_stake = Deserialize::deserialize(reader)?;
            inactive_stake_by_address.insert(staker_address, inactive_stake);
        }

        Ok(StakingContract {
            balance,
            active_validators_sorted,
            active_validators_by_id,
            inactive_stake_by_address,
            inactive_validators_by_id,
            current_epoch_parking,
            previous_epoch_parking,
            current_lost_rewards,
            previous_lost_rewards,
            current_disabled_slots,
            previous_disabled_slots,
        })
    }
}

// Not really useful traits for StakingContracts.
// TODO: Assume a single staking contract for now, i.e. all staking contracts are equal.
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
            active_validators_by_id: HashMap::new(),
            inactive_validators_by_id: Default::default(),
            current_epoch_parking: HashSet::new(),
            previous_epoch_parking: HashSet::new(),
            current_lost_rewards: Default::default(),
            previous_lost_rewards: Default::default(),
            current_disabled_slots: Default::default(),
            previous_disabled_slots: Default::default(),
            inactive_stake_by_address: HashMap::new(),
        }
    }
}

impl Clone for StakingContract {
    fn clone(&self) -> Self {
        let active_validators_by_id = HashMap::from_iter(
            self.active_validators_by_id
                .iter()
                .map(|(key, value)| (key.clone(), Arc::new(value.as_ref().clone()))),
        );
        let inactive_validators_by_id = BTreeMap::from_iter(
            self.inactive_validators_by_id
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        StakingContract {
            balance: self.balance,
            active_validators_sorted: BTreeSet::from_iter(
                active_validators_by_id.values().cloned(),
            ),
            active_validators_by_id,
            inactive_validators_by_id,
            current_epoch_parking: self.current_epoch_parking.clone(),
            previous_epoch_parking: self.previous_epoch_parking.clone(),
            current_lost_rewards: self.current_lost_rewards.clone(),
            previous_lost_rewards: self.previous_lost_rewards.clone(),
            current_disabled_slots: self.current_disabled_slots.clone(),
            previous_disabled_slots: self.previous_disabled_slots.clone(),
            inactive_stake_by_address: self.inactive_stake_by_address.clone(),
        }
    }
}
