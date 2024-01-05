use std::collections::{BTreeMap, BTreeSet};

use nimiq_account::{punished_slots::PunishedSlots, *};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_collections::BitSet;
use nimiq_database::{traits::Database, volatile::VolatileDatabase};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey};
use nimiq_primitives::{
    account::AccountError,
    coin::Coin,
    policy::Policy,
    slots_allocation::{JailedValidator, PenalizedSlot},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_transaction::{
    account::staking_contract::IncomingStakingTransactionData, inherent::Inherent, SignatureProof,
};
use nimiq_utils::key_rng::SecureGenerate;

use super::*;

fn revert_penalize_inherent(
    staking_contract: &mut StakingContract,
    data_store: DataStoreWrite,
    inherent: &Inherent,
    block_state: &BlockState,
    receipt: Option<AccountReceipt>,
    validator_address: &Address,
    slot: u16,
    inactive_from: u32,
) {
    let newly_deactivated = !staking_contract
        .active_validators
        .contains_key(validator_address);

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    staking_contract
        .revert_inherent(
            inherent,
            block_state,
            receipt,
            data_store,
            &mut inherent_logger,
        )
        .expect("Failed to revert inherent");

    let mut offense_event_block = block_state.number;
    if let Inherent::Penalize { ref slot } = inherent {
        offense_event_block = slot.offense_event_block;
    }

    assert_eq!(
        logs,
        vec![
            Log::Penalize {
                validator_address: validator_address.clone(),
                offense_event_block,
                slot,
                newly_deactivated,
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from
            },
        ]
    );

    assert!(staking_contract
        .punished_slots
        .current_batch_punished_slots_map()
        .get(validator_address)
        .is_none());
    assert!(!staking_contract
        .punished_slots
        .previous_batch_punished_slots()
        .contains(slot as usize));
}

// The following code is kept as a reference on how to generate the constants.
#[test]
#[ignore]
fn generate_contract_2() {
    let mut active_validators = BTreeMap::new();
    active_validators.insert(
        Address::from([0u8; 20]),
        Coin::from_u64_unchecked(300_000_000),
    );

    let mut current_punished_slots = BTreeMap::new();
    let mut set = BTreeSet::new();
    set.insert(0);
    set.insert(10);

    current_punished_slots.insert(Address::from([1u8; 20]), set);

    let mut previous_punished_slots = BitSet::default();
    previous_punished_slots.insert(100);
    previous_punished_slots.insert(101);
    previous_punished_slots.insert(102);
    previous_punished_slots.insert(104);

    let punished_slots = PunishedSlots::new(current_punished_slots, previous_punished_slots);

    let contract = StakingContract {
        balance: Coin::from_u64_unchecked(300_000_000),
        active_validators,
        punished_slots,
    };

    assert_eq!(&hex::encode(contract.serialize_to_vec()), "");
}

#[test]
fn it_can_de_serialize_a_staking_contract() {
    let contract_1 = StakingContract::default();
    let contract_1a: StakingContract =
        Deserialize::deserialize_from_vec(&contract_1.serialize_to_vec()).unwrap();

    assert_eq!(contract_1, contract_1a);

    let balance = Coin::from_u64_unchecked(300_000_000);

    let mut active_validators: BTreeMap<Address, Coin> = BTreeMap::new();
    active_validators.insert(Address::START_ADDRESS, Coin::MAX);

    let mut current_punished_slots = BTreeMap::new();
    let mut set = BTreeSet::new();
    set.insert(1);
    set.insert(2);

    current_punished_slots.insert(Address::START_ADDRESS, set);

    let mut previous_punished_slots = BitSet::default();
    previous_punished_slots.insert(1);
    previous_punished_slots.insert(2);
    previous_punished_slots.insert(3);
    previous_punished_slots.insert(4);

    let punished_slots = PunishedSlots::new(current_punished_slots, previous_punished_slots);

    let contract_2 = StakingContract {
        balance,
        active_validators,
        punished_slots,
    };
    let contract_2a: StakingContract =
        Deserialize::deserialize_from_vec(&contract_2.serialize_to_vec()).unwrap();

    assert_eq!(contract_2, contract_2a);
}

#[test]
fn can_get_validator() {
    let validator_setup = ValidatorSetup::new(Some(150_000_000));
    let data_store = validator_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = validator_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let _write = data_store.write(&mut db_txn);

    assert_eq!(
        validator_setup.staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000 + Policy::VALIDATOR_DEPOSIT)
    );

    let validator = validator_setup
        .staking_contract
        .get_validator(
            &data_store.read(&db_txn),
            &validator_setup.validator_address,
        )
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(150_000_000 + Policy::VALIDATOR_DEPOSIT)
    );
}

#[test]
fn create_validator_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(1, 1);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = StakingContract::default();
    let validator_address = validator_address();

    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let voting_keypair = bls_key_pair(VALIDATOR_VOTING_SECRET_KEY);
    let reward_address = Address::from([3u8; 20]);

    assert_eq!(voting_key.uncompress().unwrap(), voting_keypair.public_key);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
            proof_of_knowledge: voting_keypair
                .sign(&voting_key.serialize_to_vec())
                .compress(),
            reward_address: reward_address.clone(),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        Policy::VALIDATOR_DEPOSIT,
        &cold_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address,
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);
    assert_eq!(validator.inactive_from, None);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the validator already exists.
    let block_state = BlockState::new(2, 2);
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::AlreadyExistentAddress {
            address: validator_address.clone()
        })
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");
    assert_eq!(
        tx_logger.logs,
        vec![Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address,
        }]
    );

    assert_eq!(
        staking_contract.get_validator(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(staking_contract.balance, Coin::ZERO);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        None
    );
}

#[test]
fn update_validator_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut rng = test_rng(false);
    let mut validator_setup = ValidatorSetup::new(Some(150_000_000));
    let data_store = validator_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = validator_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let block_state = BlockState::new(2, 2);
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let new_voting_keypair = BlsKeyPair::generate(&mut rng);
    let new_reward_address = Some(Address::from([77u8; 20]));

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(PublicKey::from([88u8; 32])),
            new_voting_key: Some(new_voting_keypair.public_key.compress()),
            new_reward_address: new_reward_address.clone(),
            new_signal_data: Some(Some(Blake2bHash::default())),
            new_proof_of_knowledge: Some(
                new_voting_keypair
                    .sign(&new_voting_keypair.public_key.serialize_to_vec())
                    .compress(),
            ),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = validator_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let old_signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let old_voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let old_reward_address = validator_setup.validator_address.clone();

    let expected_receipt = UpdateValidatorReceipt {
        old_signing_key,
        old_voting_key: old_voting_key.clone(),
        old_reward_address: old_reward_address.clone(),
        old_signal_data: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateValidator {
            validator_address: validator_setup.validator_address.clone(),
            old_reward_address: old_reward_address.clone(),
            new_reward_address: new_reward_address.clone(),
        }]
    );

    let validator = validator_setup
        .staking_contract
        .get_validator(
            &data_store.read(&db_txn),
            &validator_setup.validator_address,
        )
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_setup.validator_address);
    assert_eq!(validator.signing_key, PublicKey::from([88u8; 32]));
    assert_eq!(
        validator.voting_key,
        new_voting_keypair.public_key.compress()
    );
    assert_eq!(validator.reward_address, Address::from([77u8; 20]));
    assert_eq!(validator.signal_data, Some(Blake2bHash::default()));
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_from, None);

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    validator_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateValidator {
            validator_address: validator_setup.validator_address.clone(),
            old_reward_address: old_reward_address.clone(),
            new_reward_address,
        }]
    );

    let validator = validator_setup
        .staking_contract
        .get_validator(
            &data_store.read(&db_txn),
            &validator_setup.validator_address,
        )
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_setup.validator_address);
    assert_eq!(validator.signing_key, old_signing_key);
    assert_eq!(validator.voting_key, old_voting_key);
    assert_eq!(validator.reward_address, old_reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_from, None);

    // Try with a non-existent validator.
    let fake_keypair = ed25519_key_pair(NON_EXISTENT_PRIVATE_KEY);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(PublicKey::from([88u8; 32])),
            new_voting_key: Some(new_voting_keypair.public_key.compress()),
            new_reward_address: Some(Address::from([77u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            new_proof_of_knowledge: Some(
                new_voting_keypair
                    .sign(&new_voting_keypair.public_key.serialize_to_vec())
                    .compress(),
            ),
            proof: SignatureProof::default(),
        },
        0,
        &fake_keypair,
    );

    assert_eq!(
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty(),
            ),
        Err(AccountError::NonExistentAddress {
            address: non_existent_address()
        })
    );
}

#[test]
fn deactivate_validator_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut validator_setup = ValidatorSetup::new(Some(150_000_000));
    let data_store = validator_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = validator_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let block_state = BlockState::new(2, 2);

    let validator_address = validator_setup.validator_address;
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = validator_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![Log::DeactivateValidator {
            validator_address: validator_address.clone(),
            inactive_from: Policy::election_block_after(block_state.number)
        }]
    );

    let validator = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.inactive_from,
        Some(Policy::election_block_after(block_state.number))
    );

    assert!(!validator_setup
        .staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with an already inactive validator.
    assert_eq!(
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::InvalidForRecipient)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    validator_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::DeactivateValidator {
            validator_address: validator_address.clone(),
            inactive_from: Policy::election_block_after(block_state.number)
        }]
    );

    let validator = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_from, None);

    assert!(validator_setup
        .staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with a non-existent validator.
    let fake_address = non_existent_address();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: fake_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::NonExistentAddress {
            address: fake_address
        })
    );

    // Try with a wrong signature.
    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &invalid_tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::InvalidSignature)
    );
}

#[test]
fn retire_validator_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut validator_setup = ValidatorSetup::new(Some(150_000_000));
    let data_store = validator_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = validator_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let block_state = BlockState::new(2, 2);

    let validator_address = validator_setup.validator_address;
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );
    let mut tx_logs = TransactionLog::empty();
    let receipt = validator_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logs,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireValidatorReceipt { was_active: true };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logs.logs,
        [
            Log::RetireValidator {
                validator_address: validator_address.clone()
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: Policy::election_block_after(block_state.number)
            },
        ]
    );

    assert!(!validator_setup
        .staking_contract
        .active_validators
        .contains_key(&validator_address));

    let validator = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(
        validator.inactive_from,
        Some(Policy::election_block_after(block_state.number))
    );
    assert!(validator.retired);

    // Try with an already retired validator.
    assert_eq!(
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::InvalidForRecipient)
    );

    // Revert the retire tx.
    let mut tx_logger = TransactionLog::empty();
    validator_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: Policy::election_block_after(block_state.number)
            },
            Log::RetireValidator {
                validator_address: validator_address.clone()
            }
        ]
    );

    assert!(validator_setup
        .staking_contract
        .active_validators
        .contains_key(&validator_address));

    let validator = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert!(validator.inactive_from.is_none());
    assert!(!validator.retired);

    // Try with a wrong signature.
    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &invalid_tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::NonExistentAddress {
            address: Address::from(&signing_keypair.public)
        })
    );
}

#[test]
fn delete_validator_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut validator_setup = ValidatorSetup::new(Some(150_000_000));
    let data_store = validator_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = validator_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let block_state = BlockState::new(2, 2);

    let validator_address = validator_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work when the validator is still active.
    let tx = make_delete_validator_transaction();

    assert_eq!(
        validator_setup
            .staking_contract
            .commit_outgoing_transaction(
                &tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::InvalidForSender)
    );

    // Deactivate validator.
    let deactivate_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY),
    );

    validator_setup
        .staking_contract
        .commit_incoming_transaction(
            &deactivate_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
    let effective_deactivation_block = Policy::election_block_after(block_state.number);

    // Doesn't work with a deactivated but not retired validator.
    let block_state = BlockState::new(effective_deactivation_block + 1, 1000);

    assert_eq!(
        validator_setup
            .staking_contract
            .commit_outgoing_transaction(
                &tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::InvalidForSender)
    );

    // Retire the validator.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_PRIVATE_KEY),
    );

    validator_setup
        .staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
    let inactive_release = Policy::block_after_reporting_window(effective_deactivation_block);

    // Doesn't work if the cooldown hasn't expired.
    assert_eq!(
        validator_setup
            .staking_contract
            .commit_outgoing_transaction(
                &tx,
                &BlockState::new(inactive_release - 1, 999),
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty()
            ),
        Err(AccountError::InvalidForSender)
    );

    // Works in the valid case.

    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let reward_address = validator_address.clone();
    let staker_address = staker_address();

    let block_state = BlockState::new(inactive_release, 1000);

    let mut tx_logger = TransactionLog::empty();
    let receipt = validator_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = DeleteValidatorReceipt {
        signing_key,
        voting_key: voting_key.clone(),
        reward_address: reward_address.clone(),
        signal_data: None,
        inactive_from: effective_deactivation_block,
        jailed_from: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::DeleteValidator {
                validator_address: validator_address.clone(),
                reward_address: reward_address.clone(),
            }
        ]
    );

    assert_eq!(
        validator_setup
            .staking_contract
            .get_validator(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(
        validator_setup
            .staking_contract
            .get_tombstone(&data_store.read(&db_txn), &validator_address),
        Some(Tombstone {
            remaining_stake: Coin::from_u64_unchecked(150_000_000),
            num_remaining_stakers: 1,
        })
    );

    let staker = validator_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.delegation, Some(validator_address.clone()));

    assert_eq!(
        validator_setup.staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000)
    );

    // Revert the delete transaction.
    let mut tx_logger = TransactionLog::empty();
    validator_setup
        .staking_contract
        .revert_outgoing_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::DeleteValidator {
                validator_address: validator_address.clone(),
                reward_address: reward_address.clone(),
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
        ]
    );

    let validator = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_from, Some(effective_deactivation_block));
    assert!(validator.retired);

    assert_eq!(
        validator_setup
            .staking_contract
            .get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(
        validator_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
}

#[test]
fn reward_inherents_not_allowed() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    let inherent = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: validator_address,
        value: Coin::ZERO,
    };

    assert_eq!(
        staking_contract.commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty()
        ),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn jail_inherents_work() {
    let genesis_block_number = Policy::genesis_block_number();
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2 + genesis_block_number, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // Prepare some data.
    let slot = PenalizedSlot {
        slot: 0,
        validator_address: validator_address.clone(),
        offense_event_block: 1 + genesis_block_number,
    };

    let inherent = Inherent::Penalize { slot: slot.clone() };

    // Works in current epoch, current batch case.
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    let expected_receipt = PenalizeReceipt {
        newly_deactivated: true,
        newly_punished_previous_batch: false,
        newly_punished_current_batch: true,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    let inactive_from = Policy::election_block_after(block_state.number);
    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from
            },
            Log::Penalize {
                validator_address: slot.validator_address.clone(),
                offense_event_block: slot.offense_event_block,
                slot: slot.slot,
                newly_deactivated: true,
            },
        ]
    );

    assert!(staking_contract
        .punished_slots
        .current_batch_punished_slots_map()
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(!staking_contract
        .punished_slots
        .previous_batch_punished_slots()
        .contains(slot.slot as usize));

    revert_penalize_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
        inactive_from,
    );

    // Works in current epoch, previous batch case.
    let block_state = BlockState::new(Policy::blocks_per_batch() + 1 + genesis_block_number, 500);

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    let expected_receipt = PenalizeReceipt {
        newly_deactivated: true,
        newly_punished_previous_batch: true,
        newly_punished_current_batch: true,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    let inactive_from = Policy::election_block_after(block_state.number);
    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: slot.validator_address.clone(),
                inactive_from
            },
            Log::Penalize {
                validator_address: slot.validator_address.clone(),
                offense_event_block: 1 + genesis_block_number,
                slot: slot.slot,
                newly_deactivated: true,
            },
        ]
    );

    assert!(staking_contract
        .punished_slots
        .current_batch_punished_slots_map()
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .punished_slots
        .previous_batch_punished_slots()
        .contains(slot.slot as usize));

    revert_penalize_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
        inactive_from,
    );

    // Works in previous epoch, previous batch case.
    let block_state = BlockState::new(Policy::blocks_per_epoch() + 1 + genesis_block_number, 1000);
    let slot = PenalizedSlot {
        slot: 0,
        validator_address: validator_address.clone(),
        offense_event_block: Policy::blocks_per_epoch() - 1 + genesis_block_number,
    };

    let inherent = Inherent::Penalize { slot: slot.clone() };

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    let expected_receipt = PenalizeReceipt {
        newly_deactivated: true,
        newly_punished_previous_batch: true,
        newly_punished_current_batch: false,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    let inactive_from = Policy::election_block_after(block_state.number);
    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: slot.validator_address.clone(),
                inactive_from
            },
            Log::Penalize {
                validator_address: slot.validator_address,
                offense_event_block: slot.offense_event_block,
                slot: slot.slot,
                newly_deactivated: true
            },
        ]
    );

    assert!(staking_contract
        .punished_slots
        .current_batch_punished_slots_map()
        .get(&validator_address)
        .is_none());
    assert!(staking_contract
        .punished_slots
        .previous_batch_punished_slots()
        .contains(slot.slot as usize));

    revert_penalize_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
        inactive_from,
    );
}

#[test]
fn finalize_batch_inherents_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(
        Policy::blocks_per_batch() + Policy::genesis_block_number(),
        500,
    );
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // Prepare the staking contract.
    let mut set = BTreeSet::default();
    set.insert(0);

    staking_contract
        .punished_slots
        .current_batch_punished_slots
        .insert(validator_address, set);
    staking_contract
        .punished_slots
        .previous_batch_punished_slots
        .insert(1);

    // Works in the valid case.
    let inherent = Inherent::FinalizeBatch;

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    assert_eq!(receipt, None);
    assert!(logs.is_empty());

    assert!(staking_contract
        .punished_slots
        .current_batch_punished_slots
        .is_empty());
    assert!(staking_contract
        .punished_slots
        .previous_batch_punished_slots
        .contains(0));

    // Cannot revert the inherent.
    assert_eq!(
        staking_contract.revert_inherent(
            &inherent,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty()
        ),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn finalize_epoch_inherents_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(
        Policy::blocks_per_epoch() + Policy::genesis_block_number(),
        1000,
    );
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // Pre populate the previous epoch and batch related sets.
    // To test proper behaviour upon epoch finalization.
    staking_contract
        .punished_slots
        .previous_batch_punished_slots
        .insert(10);
    staking_contract
        .punished_slots
        .current_batch_punished_slots
        .insert(Address::END_ADDRESS, BTreeSet::new());

    // Penalize the validator slot
    let offense_event_block = block_state.number - 1;
    let inherent = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            offense_event_block,
        },
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            PenalizeReceipt {
                newly_deactivated: true,
                newly_punished_previous_batch: false,
                newly_punished_current_batch: true,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: Policy::election_block_after(block_state.number)
            },
            Log::Penalize {
                validator_address: validator_address.clone(),
                offense_event_block,
                slot: 1,
                newly_deactivated: true
            },
        ]
    );

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Finalize epoch to check that the relevant sets are set properly.
    let finalize_batch_inherent = Inherent::FinalizeBatch;

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &finalize_batch_inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    assert_eq!(receipt, None);
    assert_eq!(logs, vec![]);

    assert!(staking_contract
        .punished_slots
        .current_batch_punished_slots()
        .is_empty());

    let mut bitset = BitSet::new();
    bitset.insert(1);
    assert_eq!(
        staking_contract
            .punished_slots
            .previous_batch_punished_slots(),
        &bitset
    );
    let mut set_c = BitSet::new();
    set_c.insert(1);
    assert_eq!(
        staking_contract
            .punished_slots
            .previous_batch_punished_slots(),
        &set_c
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.inactive_from,
        Some(Policy::election_block_after(block_state.number))
    );

    // Now we finalize the epoch
    let finalize_epoch_inherent = Inherent::FinalizeEpoch;
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &finalize_epoch_inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    assert_eq!(receipt, None);
    assert_eq!(logs, vec![]);

    // Cannot revert the inherent.
    assert_eq!(
        staking_contract.revert_inherent(
            &finalize_epoch_inherent,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty()
        ),
        Err(AccountError::InvalidForTarget)
    );
}

/// This test makes sure that:
/// - Validators cannot reactivate while being jailed
/// - Validators can reactivate after jail release
#[test]
fn reactivate_jail_interaction() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Create reactivate transaction
    let reactivate_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: jailed_setup.validator_address.clone(),
            proof: Default::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY),
    );

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Should fail before jail release.
    let result = jailed_setup.staking_contract.commit_incoming_transaction(
        &reactivate_tx,
        &jailed_setup.before_state_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));

    // // Should work after jail release.
    let result = jailed_setup.staking_contract.commit_incoming_transaction(
        &reactivate_tx,
        &jailed_setup.state_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert!(result.is_ok());
}

/// This test makes sure that:
/// - Validators cannot deactivate while being jailed
/// - Validators cannot deactivate despite jail release
#[test]
fn deactivate_jail_interaction() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Create reactivate transaction
    let deactivate_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: jailed_setup.validator_address.clone(),
            proof: Default::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY),
    );

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Should fail before jail release.
    let result = jailed_setup.staking_contract.commit_incoming_transaction(
        &deactivate_tx,
        &jailed_setup.before_state_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));

    // Should fail after jail release.
    let result = jailed_setup.staking_contract.commit_incoming_transaction(
        &deactivate_tx,
        &jailed_setup.state_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));
}

/// This test makes sure that:
/// - Validators cannot be deleted while being jailed
/// - Validators can be deleted after jail release
#[test]
fn delete_jail_interaction() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Retire validator
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);
    jailed_setup
        .staking_contract
        .retire_validator(
            &mut staking_contract_store,
            &jailed_setup.validator_address,
            0,
            &mut TransactionLog::empty(),
        )
        .unwrap();

    // Create delete transaction
    let delete_tx = make_delete_validator_transaction();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Should fail before jail release.
    let result = jailed_setup.staking_contract.commit_outgoing_transaction(
        &delete_tx,
        &jailed_setup.before_state_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForSender));

    // Should work after jail release.
    let result = jailed_setup.staking_contract.commit_outgoing_transaction(
        &delete_tx,
        &jailed_setup.state_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert!(result.is_ok());
}

// Jailing an active validator and reverting it
#[test]
fn jail_and_revert() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 2;
    let block_state = BlockState::new(block_number, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Prepare jail inherent.
    let inherent = Inherent::Jail {
        jailed_validator: JailedValidator {
            slots: 1..2,
            validator_address: validator_address.clone(),
            offense_event_block: 1,
        },
        new_epoch_slot_range: None,
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Commit jail inherent and thus jail validator.
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    let old_previous_batch_punished_slots = BitSet::default();
    let old_current_batch_punished_slots = None;
    let old_jailed_from = None;
    assert_eq!(
        receipt,
        Some(
            JailReceipt {
                newly_deactivated: true,
                old_previous_batch_punished_slots,
                old_current_batch_punished_slots,
                old_jailed_from
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jailed_from: block_state.number
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: Policy::election_block_after(block_state.number)
            },
            Log::Jail {
                validator_address: validator_address.clone(),
                event_block: 1,
                newly_jailed: true
            },
        ]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert_eq!(validator.jailed_from, Some(block_state.number));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert jail inherent and thus jail validator should be reverted.
    staking_contract
        .revert_inherent(
            &inherent,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));
}

// Jailing an inactive validator and reverting it (to check that it’s still inactive)
#[test]
fn jail_inactive_and_revert() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 2;
    let block_state = BlockState::new(block_number, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // 2. Deactivate validator
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);
    let result = staking_contract.deactivate_validator(
        &mut staking_contract_store,
        &validator_address,
        &Address::from(&ed25519_public_key(VALIDATOR_SIGNING_KEY)),
        block_number,
        &mut TransactionLog::empty(),
    );
    assert!(result.is_ok());
    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Prepare jail inherent.
    let inherent = Inherent::Jail {
        jailed_validator: JailedValidator {
            slots: 1..2,
            validator_address: validator_address.clone(),
            offense_event_block: 1,
        },
        new_epoch_slot_range: None,
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Commit jail inherent and thus jail validator.
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    let old_previous_batch_punished_slots = BitSet::default();
    let old_current_batch_punished_slots = None;
    let old_jailed_from = None;
    assert_eq!(
        receipt,
        Some(
            JailReceipt {
                newly_deactivated: false,
                old_previous_batch_punished_slots,
                old_current_batch_punished_slots,
                old_jailed_from
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jailed_from: block_state.number
            },
            Log::Jail {
                validator_address: validator_address.clone(),
                event_block: 1,
                newly_jailed: true
            },
        ]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert_eq!(validator.jailed_from, Some(block_state.number));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert jail inherent and thus jail validator should be reverted.
    // The deactivate state should remain.
    staking_contract
        .revert_inherent(
            &inherent,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));
}

// Validator can be jailed twice and counter resets + revert works as expected
#[test]
fn can_jail_twice() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Prepare jail inherent.
    let second_jail_block_state = BlockState::new(3, 200);
    let inherent = Inherent::Jail {
        jailed_validator: JailedValidator {
            validator_address: jailed_setup.validator_address.clone(),
            offense_event_block: second_jail_block_state.number,
            slots: 0..5,
        },
        new_epoch_slot_range: None,
    };

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Commit jail inherent and thus jail validator.
    let receipt = jailed_setup
        .staking_contract
        .commit_inherent(
            &inherent,
            &second_jail_block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    let old_previous_batch_punished_slots = BitSet::default();
    let old_current_batch_punished_slots = None;
    let old_jailed_from = Some(jailed_setup.effective_state_block_state.number);

    assert_eq!(
        receipt,
        Some(
            JailReceipt {
                newly_deactivated: false,
                old_previous_batch_punished_slots,
                old_current_batch_punished_slots,
                old_jailed_from,
            }
            .into()
        )
    );

    assert_eq!(
        logs,
        vec![
            Log::JailValidator {
                validator_address: jailed_setup.validator_address.clone(),
                jailed_from: second_jail_block_state.number,
            },
            Log::Jail {
                validator_address: jailed_setup.validator_address.clone(),
                event_block: second_jail_block_state.number,
                newly_jailed: false
            },
        ]
    );

    // Make sure that the jail release is replaced to the new jail release block height.
    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert_eq!(validator.jailed_from, Some(second_jail_block_state.number));

    // Make sure that the validator is still deactivated.
    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));

    // Revert the second jail.
    jailed_setup
        .staking_contract
        .revert_inherent(
            &inherent,
            &second_jail_block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert_eq!(
        validator.jailed_from,
        Some(jailed_setup.effective_state_block_state.number)
    );

    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));
}

/// Retire jailed validator and check revert
#[test]
fn can_retire_jailed_validator() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Prepare retire.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: Default::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_PRIVATE_KEY),
    );

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Retire jailed validator.
    let receipt = jailed_setup
        .staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &jailed_setup.before_state_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    assert_eq!(
        receipt,
        Some(RetireValidatorReceipt { was_active: false }.into())
    );

    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert!(validator.retired);

    jailed_setup
        .staking_contract
        .revert_incoming_transaction(
            &retire_tx,
            &jailed_setup.before_state_release_block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert!(!validator.retired);
    assert!(validator.inactive_from.is_some());
    assert_eq!(
        validator.jailed_from,
        Some(jailed_setup.effective_state_block_state.number)
    );
    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));
}

// Penalizing an active validator and reverting it
#[test]
fn penalize_and_revert_twice() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 5;
    let block_state = BlockState::new(block_number, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Prepare penalty.
    let inherent = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            offense_event_block: 1,
        },
    };

    let inherent2 = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            offense_event_block: 2,
        },
    };

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // First penalty.
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            PenalizeReceipt {
                newly_deactivated: true,
                newly_punished_previous_batch: false,
                newly_punished_current_batch: true,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: Policy::election_block_after(block_state.number)
            },
            Log::Penalize {
                validator_address: validator_address.clone(),
                offense_event_block: 1,
                slot: 1,
                newly_deactivated: true
            },
        ]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Second penalty.
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    let receipt2 = staking_contract
        .commit_inherent(
            &inherent2,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt2,
        Some(
            PenalizeReceipt {
                newly_deactivated: false,
                newly_punished_previous_batch: false,
                newly_punished_current_batch: false,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![Log::Penalize {
            validator_address: validator_address.clone(),
            offense_event_block: 2,
            slot: 1,
            newly_deactivated: false
        },]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert second penalize.
    // The deactivate state should remain.
    staking_contract
        .revert_inherent(
            &inherent2,
            &block_state,
            receipt2,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert first penalize.
    staking_contract
        .revert_inherent(
            &inherent,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));
}

// Penalizing an inactive validator and reverting it (to check that it’s still inactive)
#[test]
fn penalize_inactive_and_revert() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 2;
    let block_state = BlockState::new(block_number, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // 2. Deactivate validator
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);
    let result = staking_contract.deactivate_validator(
        &mut staking_contract_store,
        &validator_address,
        &Address::from(&ed25519_public_key(VALIDATOR_SIGNING_KEY)),
        block_number,
        &mut TransactionLog::empty(),
    );
    assert!(result.is_ok());
    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Prepare penalty.
    let inherent = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            offense_event_block: 1,
        },
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Penalize.
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            PenalizeReceipt {
                newly_deactivated: false,
                newly_punished_previous_batch: false,
                newly_punished_current_batch: true
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![Log::Penalize {
            validator_address: validator_address.clone(),
            offense_event_block: 1,
            slot: 1,
            newly_deactivated: false
        }]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert penalize.
    // The deactivate state should remain.
    staking_contract
        .revert_inherent(
            &inherent,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));
}

// Jailing a penalized validator and reverting it
#[test]
fn penalize_and_jail_and_revert_twice() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 5;
    let block_state = BlockState::new(block_number, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Prepare penalty.
    let inherent = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            offense_event_block: 1,
        },
    };

    let inherent2 = Inherent::Jail {
        jailed_validator: JailedValidator {
            slots: 1..3,
            validator_address: validator_address.clone(),
            offense_event_block: 2,
        },
        new_epoch_slot_range: None,
    };

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // First: penalty.
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            PenalizeReceipt {
                newly_deactivated: true,
                newly_punished_previous_batch: false,
                newly_punished_current_batch: true,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: Policy::election_block_after(block_state.number)
            },
            Log::Penalize {
                validator_address: validator_address.clone(),
                offense_event_block: 1,
                slot: 1,
                newly_deactivated: true
            },
        ]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Second: jail.
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    let receipt2 = staking_contract
        .commit_inherent(
            &inherent2,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    let old_previous_batch_punished_slots = BitSet::default();
    let mut old_current_batch_punished_slots = BTreeSet::new();
    old_current_batch_punished_slots.insert(1);
    let old_current_batch_punished_slots = Some(old_current_batch_punished_slots);
    let old_jailed_from = None;
    assert_eq!(
        receipt2,
        Some(
            JailReceipt {
                newly_deactivated: false,
                old_previous_batch_punished_slots,
                old_current_batch_punished_slots,
                old_jailed_from,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jailed_from: block_state.number,
            },
            Log::Jail {
                validator_address: validator_address.clone(),
                event_block: 2,
                newly_jailed: true
            },
        ]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert_eq!(validator.jailed_from, Some(block_state.number));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert jail.
    // The deactivate state should remain.
    staking_contract
        .revert_inherent(
            &inherent2,
            &block_state,
            receipt2,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert first penalize.
    staking_contract
        .revert_inherent(
            &inherent,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jailed_from.is_none());

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));
}

// Penalizing a jailed validator
#[test]
fn jail_and_penalize_and_revert_twice() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Prepare jail inherent.
    let penalty_block_state = BlockState::new(2, 200);
    let inherent = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: 1,
            validator_address: jailed_setup.validator_address.clone(),
            offense_event_block: penalty_block_state.number,
        },
    };

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Penalize slot.
    let receipt = jailed_setup
        .staking_contract
        .commit_inherent(
            &inherent,
            &penalty_block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            PenalizeReceipt {
                newly_deactivated: false,
                newly_punished_previous_batch: false,
                newly_punished_current_batch: true,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![Log::Penalize {
            validator_address: jailed_setup.validator_address.clone(),
            offense_event_block: penalty_block_state.number,
            slot: 1,
            newly_deactivated: false
        }]
    );

    // Make sure that the jail release is not changed by the penalty.
    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert_eq!(
        validator.jailed_from,
        Some(jailed_setup.effective_state_block_state.number)
    );

    // Make sure that the validator is still deactivated.
    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));

    // Revert the penalty.
    jailed_setup
        .staking_contract
        .revert_inherent(
            &inherent,
            &penalty_block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty(),
        )
        .expect("Failed to revert inherent");

    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert_eq!(
        validator.jailed_from,
        Some(jailed_setup.effective_state_block_state.number)
    );

    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));
}

#[test]
fn can_reserve_and_release_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let retired_setup = ValidatorSetup::setup_retired_validator(None);
    let data_store = retired_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = retired_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let _write = data_store.write(&mut db_txn);

    // Reserve balance for delete validator tx.
    let mut reserved_balance = ReservedBalance::new(retired_setup.validator_address.clone());

    let tx = make_delete_validator_transaction();
    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );

    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert!(result.is_ok());

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot reserve balance for further delete validator txs.
    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT * 2),
            balance: Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
        })
    );

    // Can release balance.
    let result = retired_setup.staking_contract.release_balance(
        &tx,
        &mut reserved_balance,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::ZERO);
    assert!(result.is_ok());

    // Can reserve balance for delete validator tx after released funds.
    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert!(result.is_ok());
}

#[test]
fn cannot_reserve_balance_if_value_different_than_validator_deposit() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let retired_setup = ValidatorSetup::setup_retired_validator(None);
    let data_store = retired_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = retired_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let _write = data_store.write(&mut db_txn);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot reserve with value < deposit.
    let mut reserved_balance = ReservedBalance::new(retired_setup.validator_address.clone());

    let mut tx = make_delete_validator_transaction();
    tx.value -= Coin::from_u64_unchecked(1);

    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );

    assert_eq!(reserved_balance.balance(), Coin::ZERO);
    assert_eq!(result, Err(AccountError::InvalidCoinValue));

    // Works with correct total tx value.
    tx.value += Coin::from_u64_unchecked(1);
    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn cannot_reserve_balance_if_not_released() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let retired_setup = ValidatorSetup::setup_retired_validator(None);
    let data_store = retired_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = retired_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let _write = data_store.write(&mut db_txn);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot reserve balance before cooldown.
    let mut reserved_balance = ReservedBalance::new(retired_setup.validator_address.clone());

    let tx = make_delete_validator_transaction();
    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.before_state_release_block_state,
        data_store.read(&mut db_txn),
    );

    assert_eq!(reserved_balance.balance(), Coin::ZERO);
    assert_eq!(result, Err(AccountError::InvalidForSender));

    // Works after cooldown release.
    let result = retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn cannot_reserve_balance_if_jailed() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_retired_setup = ValidatorSetup::setup_jailed_validator(None);
    let data_store = jailed_retired_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_retired_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let _write = data_store.write(&mut db_txn);

    // Retire jailed validator.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: Default::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_PRIVATE_KEY),
    );

    _ = jailed_retired_setup
        .staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &jailed_retired_setup.before_state_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot reserve balance while jailed.
    let mut reserved_balance = ReservedBalance::new(jailed_retired_setup.validator_address.clone());

    let tx = make_delete_validator_transaction();
    let result = jailed_retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &jailed_retired_setup.before_state_release_block_state,
        data_store.read(&mut db_txn),
    );

    assert_eq!(reserved_balance.balance(), Coin::ZERO);
    assert_eq!(result, Err(AccountError::InvalidForSender));

    // Works after jail release.
    let result = jailed_retired_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &jailed_retired_setup.state_release_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(result, Ok(()));
}
