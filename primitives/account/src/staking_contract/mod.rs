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

use crate::staking_contract::validator::Validator;
use crate::AccountError;

pub mod receipts;
pub mod staker;
pub mod traits;
pub mod validator;

/// The struct representing the staking contract. The staking contract is a special contract that
/// handles many functions related to validators and staking.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StakingContract {
    // The total amount of coins staked.
    pub balance: Coin,
    // A list of active validators IDs (i.e. are eligible to receive slots).
    pub active_validators: HashSet<ValidatorId>,
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
    // The validator slots, searchable by the validator ID, that are disabled (i.e. are no
    // longer eligible to produce blocks) currently.
    pub current_disabled_slots: HashMap<ValidatorId, BTreeSet<u16>>,
    // The validator slots, searchable by the validator ID, that were disabled (i.e. are no
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
