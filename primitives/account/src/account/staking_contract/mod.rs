use std::collections::btree_set::BTreeSet;
use std::collections::BTreeMap;

use beserial::{Deserialize, Serialize};
use nimiq_collections::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots::{Validators, ValidatorsBuilder},
};
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_trie::trie::IncompleteTrie;
use nimiq_vrf::{AliasMethod, VrfSeed, VrfUseCase};

use crate::{Account, AccountsTrie};

pub use receipts::*;
pub use staker::Staker;
pub use validator::Validator;

use crate::account::staking_contract::store::{
    StakingContractStoreRead, StakingContractStoreReadOps,
};
use crate::data_store::DataStoreRead;

mod receipts;
mod staker;
mod store;
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
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StakingContract {
    // The total amount of coins staked (also includes validators deposits).
    pub balance: Coin,
    // The list of active validators addresses (i.e. are eligible to receive slots) and their
    // corresponding balances.
    #[beserial(len_type(u32))]
    pub active_validators: BTreeMap<Address, Coin>,
    // The validators that were parked during the current epoch.
    #[beserial(len_type(u32))]
    pub parked_set: BTreeSet<Address>,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the current batch.
    pub current_lost_rewards: BitSet,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the previous batch.
    pub previous_lost_rewards: BitSet,
    // The validator slots, searchable by the validator address, that are disabled (i.e. are no
    // longer eligible to produce blocks) currently.
    #[beserial(len_type(u32))]
    pub current_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
    // The validator slots, searchable by the validator address, that were disabled (i.e. are no
    // longer eligible to produce blocks) during the previous batch.
    #[beserial(len_type(u32))]
    pub previous_disabled_slots: BTreeMap<Address, BTreeSet<u16>>,
}

impl StakingContract {
    /// Get a validator given its address, if it exists.
    pub fn get_validator(
        &self,
        data_store: &DataStoreRead,
        address: &Address,
    ) -> Option<Validator> {
        StakingContractStoreRead::new(data_store).get_validator(address)
    }

    /// Get a staker given its address, if it exists.
    pub fn get_staker(&self, data_store: &DataStoreRead, address: &Address) -> Option<Staker> {
        StakingContractStoreRead::new(data_store).get_staker(address)
    }

    /// Get a list containing the addresses of all stakers that are delegating for a given validator.
    pub fn get_stakers_for_validator(
        &self,
        _data_store: &DataStoreRead,
        _address: &Address,
    ) -> Vec<Address> {
        todo!()
    }

    /// Given a seed, it randomly distributes the validator slots across all validators. It is
    /// used to select the validators for the next epoch.
    pub fn select_validators(&self, data_store: &DataStoreRead, seed: &VrfSeed) -> Validators {
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

    //     /// Checks if a given sender can pay the transaction.
    //     pub fn can_pay_tx(
    //         accounts_tree: &AccountsTrie,
    //         db_txn: &DBTransaction,
    //         tx_proof: OutgoingStakingTransactionProof,
    //         tx_value: Coin,
    //         block_height: u32,
    //     ) -> bool {
    //         match tx_proof {
    //             OutgoingStakingTransactionProof::DeleteValidator { proof } => {
    //                 // Get the validator address from the proof.
    //                 let validator_address = proof.compute_signer();
    //
    //                 // Get the validator.
    //                 let validator =
    //                     match StakingContract::get_validator(accounts_tree, db_txn, &validator_address)
    //                     {
    //                         Some(v) => v,
    //                         None => {
    //                             warn!(
    //                                 "Cannot pay fees for transaction because validator doesn't exist."
    //                             );
    //                             return false;
    //                         }
    //                     };
    //
    //                 // If the fee is larger than the validator deposit then this won't work.
    //                 if tx_value > validator.deposit {
    //                     warn!(
    //     "Cannot pay fees for transaction because fee is larger than validator deposit."
    // );
    //                     return false;
    //                 }
    //
    //                 // Check that the validator has been inactive for long enough.
    //                 match validator.inactivity_flag {
    //                     None => {
    //                         warn!("Cannot pay fees for transaction because validator is still active.");
    //                         return false;
    //                     }
    //                     Some(time) => {
    //                         if block_height
    //                             <= policy::election_block_after(time) + policy::BLOCKS_PER_BATCH
    //                         {
    //                             warn!(
    //                     "Cannot pay fees for transaction because validator hasn't been inactive for long enough."
    //                 );
    //                             return false;
    //                         }
    //                     }
    //                 }
    //             }
    //             OutgoingStakingTransactionProof::Unstake { proof } => {
    //                 // Get the staker address from the proof.
    //                 let staker_address = proof.compute_signer();
    //
    //                 // Get the staker.
    //                 let staker =
    //                     match StakingContract::get_staker(accounts_tree, db_txn, &staker_address) {
    //                         Some(v) => v,
    //                         None => {
    //                             warn!("Cannot pay fees for transaction because staker doesn't exist.");
    //                             return false;
    //                         }
    //                     };
    //
    //                 // Check that the balance is enough to pay the fee.
    //                 if tx_value > staker.balance {
    //                     warn!(
    //                     "Cannot pay fees for transaction because fee is larger than staker balance."
    //                 );
    //                     return false;
    //                 }
    //             }
    //         }
    //
    //         true
    //     }
    //
    //     /// Checks if a given creation transaction will succeed.
    //     pub fn can_create(
    //         accounts_tree: &AccountsTrie,
    //         db_txn: &DBTransaction,
    //         tx_data: IncomingStakingTransactionData,
    //     ) -> bool {
    //         match tx_data {
    //             IncomingStakingTransactionData::CreateValidator { proof, .. } => {
    //                 // Get the validator address from the proof.
    //                 let validator_address = proof.compute_signer();
    //
    //                 // If the validator already exists then the transaction would fail.
    //                 if StakingContract::get_validator(accounts_tree, db_txn, &validator_address)
    //                     .is_some()
    //                 {
    //                     warn!("Cannot create validator because validator already exists.");
    //                     return false;
    //                 };
    //             }
    //             IncomingStakingTransactionData::CreateStaker { proof, delegation } => {
    //                 // Get the staker address from the proof.
    //                 let staker_address = proof.compute_signer();
    //
    //                 // If the staker already exists then the transaction would fail.
    //                 if StakingContract::get_staker(accounts_tree, db_txn, &staker_address).is_some() {
    //                     warn!("Cannot create staker because staker already exists.");
    //                     return false;
    //                 };
    //
    //                 // Verify we have a valid delegation address (if present)
    //                 if let Some(delegation) = delegation {
    //                     let key = StakingContract::get_key_validator(&delegation);
    //                     if accounts_tree.get::<Account>(db_txn, &key).is_none() {
    //                         return false;
    //                     }
    //                 }
    //             }
    //             _ => {}
    //         }
    //
    //         true
    //     }
}
