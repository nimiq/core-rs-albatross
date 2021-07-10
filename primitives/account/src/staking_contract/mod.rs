use log::error;
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
use nimiq_database::{Database, Environment, Transaction as DBTransaction, WriteTransaction};
use nimiq_keys::Address;
use nimiq_primitives::account::ValidatorId;
use nimiq_primitives::slots::{Validators, ValidatorsBuilder};
use nimiq_primitives::{coin::Coin, policy};
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};
use validator::Validator;

use crate::{Account, AccountError, AccountsTree};
use nimiq_trie::key_nibbles::KeyNibbles;
use staker::Staker;

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
    // A list of active validators IDs (i.e. are eligible to receive slots) and their corresponding
    // stakes/balances.
    pub active_validators: HashMap<ValidatorId, Coin>,
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

        KeyNibbles::from(&bytes)
    }

    pub fn get_key_validator(validator_id: &ValidatorId) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(42);
        bytes.extend(
            Address::from_user_friendly_address(policy::STAKING_CONTRACT_ADDRESS)
                .expect("Couldn't parse the staking contract address!")
                .as_bytes(),
        );
        bytes.push(StakingContract::PATH_VALIDATORS_LIST);
        bytes.extend(validator_id.as_bytes());
        bytes.push(StakingContract::PATH_VALIDATOR_MAIN);

        KeyNibbles::from(&bytes)
    }

    pub fn get_key_validator_staker(
        validator_id: &ValidatorId,
        staker_address: &Address,
    ) -> KeyNibbles {
        let mut bytes = Vec::with_capacity(62);
        bytes.extend(
            Address::from_user_friendly_address(policy::STAKING_CONTRACT_ADDRESS)
                .expect("Couldn't parse the staking contract address!")
                .as_bytes(),
        );
        bytes.push(StakingContract::PATH_VALIDATORS_LIST);
        bytes.extend(validator_id.as_bytes());
        bytes.push(StakingContract::PATH_VALIDATOR_STAKERS_LIST);
        bytes.extend(staker_address.as_bytes());

        KeyNibbles::from(&bytes)
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

        KeyNibbles::from(&bytes)
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
        validator_id: &ValidatorId,
    ) -> Option<Validator> {
        let key = StakingContract::get_key_validator(validator_id);

        trace!(
            "Trying to fetch validator with id {} at key {}.",
            validator_id.to_string(),
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
        let mut validator_ids = Vec::with_capacity(self.active_validators.len());
        let mut validator_stakes = Vec::with_capacity(self.active_validators.len());

        debug!("Selecting validators: num_slots = {}", policy::SLOTS);

        for (id, coin) in self.active_validators.iter() {
            validator_ids.push(id);
            validator_stakes.push(u64::from(coin));
        }

        let mut rng = seed.rng(VrfUseCase::ValidatorSelection, 0);

        let lookup = AliasMethod::new(validator_stakes);

        let mut slots_builder = ValidatorsBuilder::default();

        for _ in 0..policy::SLOTS {
            let index = lookup.sample(&mut rng);

            let chosen_validator =
                StakingContract::get_validator(accounts_tree, db_txn, validator_ids[index]).expect("Couldn't find in the accounts tree a validator that was in the active validators list!");

            slots_builder.push(
                chosen_validator.id.clone(),
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

    /// Get the signature from a transaction.
    fn get_self_signer(transaction: &Transaction) -> Result<Address, AccountError> {
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        Ok(signature_proof.compute_signer())
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
