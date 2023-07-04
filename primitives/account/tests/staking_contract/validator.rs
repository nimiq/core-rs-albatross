use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryInto,
};

use nimiq_account::*;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_collections::BitSet;
use nimiq_database::{
    traits::{Database, WriteTransaction},
    volatile::VolatileDatabase,
    DatabaseProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
    slots::SlashedSlot,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionProof},
    inherent::Inherent,
    SignatureProof, Transaction,
};
use nimiq_utils::key_rng::SecureGenerate;

use super::*;

fn make_delete_validator_transaction() -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        non_existent_address(),
        AccountType::Basic,
        (Policy::VALIDATOR_DEPOSIT - 100).try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let sig = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()));

    let proof = OutgoingStakingTransactionProof::DeleteValidator { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn revert_slash_inherent(
    staking_contract: &mut StakingContract,
    data_store: DataStoreWrite,
    inherent: &Inherent,
    block_state: &BlockState,
    receipt: Option<AccountReceipt>,
    validator_address: &Address,
    slot: u16,
) {
    let newly_disabled = staking_contract
        .current_epoch_disabled_slots
        .contains_key(validator_address);
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

    let mut event_block = block_state.number;
    if let Inherent::Slash { ref slot } = inherent {
        event_block = slot.event_block;
    }

    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            },
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number),
            },
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block,
                slot,
                newly_disabled,
                newly_deactivated,
            },
        ]
    );

    assert!(!staking_contract
        .current_batch_lost_rewards
        .contains(slot as usize));
    assert!(!staking_contract
        .previous_batch_lost_rewards
        .contains(slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(validator_address)
        .is_none());
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(validator_address)
        .is_none());
}

struct JailedSetup {
    env: DatabaseProxy,
    accounts: Accounts,
    staking_contract: StakingContract,
    still_jailed_block_state: BlockState,
    jail_release_block_state: BlockState,
    validator_address: Address,
}

fn setup_jailed_validator() -> JailedSetup {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 1;
    let jail_release: u32 = Policy::block_after_jail(block_number);
    let before_jail_release: u32 = jail_release - 1;

    let still_jailed_block_state = BlockState::new(before_jail_release, 1000);
    let jail_release_block_state = BlockState::new(jail_release, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn_og = env.write_transaction();
    let mut db_txn = (&mut db_txn_og).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    // 2. Jail validator
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);
    let result = staking_contract
        .jail_validator(
            &mut staking_contract_store,
            &validator_address,
            block_number,
            jail_release,
            &mut TransactionLog::empty(),
        )
        .unwrap();
    assert_eq!(
        result,
        JailValidatorReceipt {
            newly_deactivated: true,
            old_jail_release: None
        }
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert_eq!(
        validator.jail_release,
        Some(Policy::block_after_jail(block_number))
    );

    // Make sure that the validator is still deactivated.
    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    db_txn_og.commit();

    JailedSetup {
        env,
        accounts,
        staking_contract,
        still_jailed_block_state,
        jail_release_block_state,
        validator_address,
    }
}

// The following code is kept as a reference on how to generate the constants.
#[ignore]
#[test]
fn generate_contract_2() {
    let mut active_validators = BTreeMap::new();
    active_validators.insert(
        Address::from([0u8; 20]),
        Coin::from_u64_unchecked(300_000_000),
    );

    let mut current_batch_lost_rewards = BitSet::new();
    current_batch_lost_rewards.insert(0);
    current_batch_lost_rewards.insert(10);

    let mut previous_batch_lost_rewards = BitSet::new();
    previous_batch_lost_rewards.insert(100);
    previous_batch_lost_rewards.insert(101);
    previous_batch_lost_rewards.insert(102);
    previous_batch_lost_rewards.insert(104);

    let mut b_set = BTreeSet::new();
    b_set.insert(0);
    b_set.insert(10);
    let mut current_epoch_disabled_slots = BTreeMap::new();
    current_epoch_disabled_slots.insert(Address::from([1u8; 20]), b_set);

    let mut b_set = BTreeSet::new();
    b_set.insert(100);
    b_set.insert(101);
    b_set.insert(102);
    b_set.insert(104);
    let mut previous_epoch_disabled_slots = BTreeMap::new();
    previous_epoch_disabled_slots.insert(Address::from([2u8; 20]), b_set);

    let contract = StakingContract {
        balance: Coin::from_u64_unchecked(300_000_000),
        active_validators,
        current_batch_lost_rewards,
        previous_batch_lost_rewards,
        current_epoch_disabled_slots,
        previous_epoch_disabled_slots,
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
    let mut current_batch_lost_rewards = BitSet::new();
    current_batch_lost_rewards.insert(1);
    current_batch_lost_rewards.insert(2);
    let mut active_validators: BTreeMap<Address, Coin> = BTreeMap::new();
    active_validators.insert(Address::START_ADDRESS, Coin::MAX);
    let mut previous_batch_lost_rewards = current_batch_lost_rewards.clone();
    previous_batch_lost_rewards.insert(3);
    previous_batch_lost_rewards.insert(4);
    let mut current_epoch_disabled_slots = BTreeMap::new();
    current_epoch_disabled_slots.insert(Address::START_ADDRESS, BTreeSet::new());
    let previous_epoch_disabled_slots = current_epoch_disabled_slots.clone();

    let contract_2 = StakingContract {
        balance,
        active_validators,
        current_batch_lost_rewards,
        previous_batch_lost_rewards,
        current_epoch_disabled_slots,
        previous_epoch_disabled_slots,
    };
    let contract_2a: StakingContract =
        Deserialize::deserialize_from_vec(&contract_2.serialize_to_vec()).unwrap();

    assert_eq!(contract_2, contract_2a);
}

#[test]
fn can_get_it() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000 + Policy::VALIDATOR_DEPOSIT)
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address())
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
    assert_eq!(validator.inactive_since, None);

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
    let mut rng = test_rng(false);
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let new_voting_keypair = BlsKeyPair::generate(&mut rng);
    let new_reward_address = Some(Address::from([77u8; 20]));

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
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let old_signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let old_voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let old_reward_address = validator_address.clone();

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
            validator_address: validator_address.clone(),
            old_reward_address: old_reward_address.clone(),
            new_reward_address: new_reward_address.clone(),
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
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
    assert_eq!(validator.inactive_since, None);

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
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
            validator_address: validator_address.clone(),
            old_reward_address: old_reward_address.clone(),
            new_reward_address,
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, old_signing_key);
    assert_eq!(validator.voting_key, old_voting_key);
    assert_eq!(validator.reward_address, old_reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, None);

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
        staking_contract.commit_incoming_transaction(
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
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

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
        vec![Log::DeactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
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
    assert_eq!(validator.inactive_since, Some(2));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with an already inactive validator.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
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
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
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
    assert_eq!(validator.inactive_since, None);

    assert!(staking_contract
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
        staking_contract.commit_incoming_transaction(
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
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidSignature)
    );
}

#[test]
fn reactivate_validator_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, deactivate the validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
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

    let expected_receipt = ReactivateValidatorReceipt {
        was_inactive_since: 2,
        current_epoch_disabled_slots: None,
    };

    assert_eq!(receipt, Some(expected_receipt.into()));
    assert_eq!(
        tx_logger.logs,
        vec![Log::ReactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
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
    assert_eq!(validator.inactive_since, None);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Reactivate a slashed validator.
    // Slash the validator slot.
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            event_block: Policy::blocks_per_epoch() - 1,
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
            SlashReceipt {
                newly_deactivated: true,
                newly_disabled: true,
                newly_lost_rewards: true,
                old_jail_release: None,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block: Policy::blocks_per_epoch() - 1,
                slot: 1,
                newly_disabled: true,
                newly_deactivated: true
            },
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number)
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            }
        ]
    );

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    block_state.number += Policy::block_after_jail(block_state.number);
    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let mut bitset = BTreeSet::new();
    bitset.insert(1);
    let expected_receipt = ReactivateValidatorReceipt {
        was_inactive_since: 2,
        current_epoch_disabled_slots: Some(bitset),
    };

    assert_eq!(receipt, Some(expected_receipt.into()));
    assert_eq!(
        tx_logger.logs,
        vec![Log::ReactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
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
    assert_eq!(validator.inactive_since, None);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Try with an already active validator.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
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
        vec![Log::ReactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
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
    assert_eq!(validator.inactive_since, Some(2));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with a non-existent validator.
    let fake_address = non_existent_address();

    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: fake_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
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
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidSignature)
    );

    // Try with a retired validator.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );
}

#[test]
fn retire_validator_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireValidatorReceipt { was_active: true };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert!(!staking_contract
        .current_epoch_disabled_slots
        .contains_key(&validator_address));
    assert!(!staking_contract
        .previous_epoch_disabled_slots
        .contains_key(&validator_address));

    // Try with an already retired validator.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    let mut tx_logger = TransactionLog::empty();
    staking_contract
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
                validator_address: validator_address.clone()
            },
            Log::RetireValidator {
                validator_address: validator_address.clone()
            }
        ]
    );

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with a wrong signature.
    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
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
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    // Doesn't work when the validator is still active.
    let tx = make_delete_validator_transaction();

    assert_eq!(
        staking_contract.commit_outgoing_transaction(
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
            validator_address: validator_address(),
            proof: SignatureProof::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY),
    );

    staking_contract
        .commit_incoming_transaction(
            &deactivate_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Doesn't work with a deactivated but not retired validator.
    let after_cooldown = Policy::election_block_after(2) + Policy::blocks_per_batch() + 1;
    let block_state = BlockState::new(after_cooldown, 1000);

    assert_eq!(
        staking_contract.commit_outgoing_transaction(
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

    let block_state = BlockState::new(3, 3);
    staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Doesn't work if the cooldown hasn't expired.
    assert_eq!(
        staking_contract.commit_outgoing_transaction(
            &tx,
            &BlockState::new(after_cooldown - 1, 999),
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForSender)
    );

    // Works in the valid case.
    let validator_address = validator_address();
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let reward_address = validator_address.clone();
    let staker_address = staker_address();

    let block_state = BlockState::new(after_cooldown, 1000);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
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
        inactive_since: 2,
        jail_release: None,
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
        staking_contract.get_validator(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        Some(Tombstone {
            remaining_stake: Coin::from_u64_unchecked(150_000_000),
            num_remaining_stakers: 1,
        })
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.delegation, Some(validator_address.clone()));

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000)
    );

    // Revert the delete transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
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

    let validator = staking_contract
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
    assert_eq!(validator.inactive_since, Some(2));
    assert_eq!(validator.retired, true);

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(
        staking_contract.balance,
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

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    let inherent = Inherent::Reward {
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
fn slash_inherents_work() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    // Prepare some data.
    let slot = SlashedSlot {
        slot: 0,
        validator_address: validator_address.clone(),
        event_block: 2,
    };

    let inherent = Inherent::Slash { slot: slot.clone() };

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

    let expected_receipt = SlashReceipt {
        newly_deactivated: true,
        newly_disabled: true,
        newly_lost_rewards: true,
        old_jail_release: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: slot.validator_address.clone(),
                event_block: slot.event_block,
                slot: slot.slot,
                newly_disabled: true,
                newly_deactivated: true,
            },
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number)
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            }
        ]
    );

    assert!(staking_contract
        .current_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(!staking_contract
        .previous_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
    );

    // Works in current epoch, previous batch case.
    let block_state = BlockState::new(Policy::blocks_per_batch() + 1, 500);

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

    let expected_receipt = SlashReceipt {
        newly_deactivated: true,
        newly_disabled: true,
        newly_lost_rewards: true,
        old_jail_release: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: slot.validator_address.clone(),
                event_block: 2,
                slot: slot.slot,
                newly_disabled: true,
                newly_deactivated: true,
            },
            Log::JailValidator {
                validator_address: slot.validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number)
            },
            Log::DeactivateValidator {
                validator_address: slot.validator_address.clone(),
            },
        ]
    );

    assert!(!staking_contract
        .current_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .previous_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
    );

    // Works in previous epoch, previous batch case.
    let block_state = BlockState::new(Policy::blocks_per_epoch() + 1, 1000);

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

    let expected_receipt = SlashReceipt {
        newly_deactivated: true,
        newly_disabled: false,
        newly_lost_rewards: true,
        old_jail_release: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: slot.validator_address.clone(),
                event_block: slot.event_block,
                slot: slot.slot,
                newly_disabled: false,
                newly_deactivated: true
            },
            Log::JailValidator {
                validator_address: slot.validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number)
            },
            Log::DeactivateValidator {
                validator_address: slot.validator_address,
            },
        ]
    );

    assert!(!staking_contract
        .current_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .previous_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(&validator_address)
        .is_none());
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
    );
}

#[test]
fn finalize_batch_inherents_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(Policy::blocks_per_batch(), 500);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    // Prepare the staking contract.
    staking_contract.current_batch_lost_rewards.insert(0);
    staking_contract.previous_batch_lost_rewards.insert(1);

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

    assert!(staking_contract.current_batch_lost_rewards.is_empty());
    assert!(staking_contract.previous_batch_lost_rewards.contains(0));
    assert!(staking_contract.current_epoch_disabled_slots.is_empty());
    assert!(staking_contract.previous_epoch_disabled_slots.is_empty());

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
    let block_state = BlockState::new(Policy::blocks_per_epoch(), 1000);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    // Pre polutate the previous epoch and batch related sets.
    // To test proper behaviour upon epoch finalization.
    staking_contract.previous_batch_lost_rewards.insert(10);
    staking_contract
        .previous_epoch_disabled_slots
        .insert(Address::END_ADDRESS, BTreeSet::new());

    // Slash the validator slot
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            event_block: Policy::blocks_per_epoch() - 1,
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
            SlashReceipt {
                newly_deactivated: true,
                newly_disabled: true,
                newly_lost_rewards: true,
                old_jail_release: None,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block: Policy::blocks_per_epoch() - 1,
                slot: 1,
                newly_disabled: true,
                newly_deactivated: true
            },
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number)
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            }
        ]
    );

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Finalize epoch to check that the relevant sets are set properly.
    let inherent = Inherent::FinalizeEpoch;

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
    assert_eq!(logs, vec![]);

    assert!(staking_contract.current_batch_lost_rewards.is_empty());
    assert!(staking_contract.current_epoch_disabled_slots.is_empty());

    let mut bitset = BitSet::new();
    bitset.insert(1);
    assert_eq!(staking_contract.previous_batch_lost_rewards, bitset);
    let mut set_c = BTreeSet::new();
    set_c.insert(1);
    assert_eq!(
        staking_contract
            .previous_epoch_disabled_slots
            .get(&validator_address)
            .unwrap(),
        &set_c
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.inactive_since, Some(Policy::blocks_per_epoch()));

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

/// This test makes sure that:
/// - Validators cannot reactivate while being jailed
/// - Validators can reactivate after jail release
#[test]
fn reactivate_jail_interaction() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut jailed_setup = setup_jailed_validator();
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
        &jailed_setup.still_jailed_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));

    // // Should work after jail release.
    let result = jailed_setup.staking_contract.commit_incoming_transaction(
        &reactivate_tx,
        &jailed_setup.jail_release_block_state,
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
    let mut jailed_setup = setup_jailed_validator();
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
        &jailed_setup.still_jailed_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));

    // Should fail after jail release.
    let result = jailed_setup.staking_contract.commit_incoming_transaction(
        &deactivate_tx,
        &jailed_setup.jail_release_block_state,
        data_store.write(&mut db_txn),
        &mut TransactionLog::empty(),
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));
}

// Jailing an inactive validator and reverting it (to check that it’s still inactive)
#[test]
fn jail_inactive_and_revert() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let block_number: u32 = 1;
    let block_state = BlockState::new(block_number, 1000);

    // 1. Create staking contract with validator
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

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
    // Prepare slash.
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            event_block: Policy::blocks_per_epoch() - 1,
        },
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Slash and thus jail validator.
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
            SlashReceipt {
                newly_deactivated: false,
                newly_disabled: true,
                newly_lost_rewards: true,
                old_jail_release: None,
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block: Policy::blocks_per_epoch() - 1,
                slot: 1,
                newly_disabled: true,
                newly_deactivated: false
            },
            Log::JailValidator {
                validator_address: validator_address.clone(),
                jail_release: Policy::block_after_jail(block_state.number)
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            }
        ]
    );
    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .unwrap();
    assert!(validator.jail_release.is_some());

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Revert slash and thus jail validator should be reverted.
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
    assert!(validator.jail_release.is_none());

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
    let mut jailed_setup = setup_jailed_validator();
    let data_store = jailed_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = jailed_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Prepare slash.
    let second_slash_block_state = BlockState::new(2, 200);
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: 1,
            validator_address: jailed_setup.validator_address.clone(),
            event_block: second_slash_block_state.number,
        },
    };

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);

    // Slash and thus jail validator.
    let receipt = jailed_setup
        .staking_contract
        .commit_inherent(
            &inherent,
            &second_slash_block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            SlashReceipt {
                newly_deactivated: false,
                newly_disabled: true,
                newly_lost_rewards: true,
                old_jail_release: Some(jailed_setup.jail_release_block_state.number),
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: jailed_setup.validator_address.clone(),
                event_block: second_slash_block_state.number,
                slot: 1,
                newly_disabled: true,
                newly_deactivated: false
            },
            Log::JailValidator {
                validator_address: jailed_setup.validator_address.clone(),
                jail_release: Policy::block_after_jail(second_slash_block_state.number),
            },
            Log::DeactivateValidator {
                validator_address: jailed_setup.validator_address.clone(),
            }
        ]
    );

    // Make sure that the jail release is replaced to the new jail release block height.
    let validator = jailed_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &jailed_setup.validator_address)
        .unwrap();
    assert_eq!(
        validator.jail_release,
        Some(Policy::block_after_jail(second_slash_block_state.number))
    );

    // Make sure that the validator is still deactivated.
    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));

    // Revert the second slash.
    jailed_setup
        .staking_contract
        .revert_inherent(
            &inherent,
            &second_slash_block_state,
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
        validator.jail_release,
        Some(jailed_setup.jail_release_block_state.number)
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
    let mut jailed_setup = setup_jailed_validator();
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
            &jailed_setup.still_jailed_block_state,
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
            &jailed_setup.still_jailed_block_state,
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
    assert!(validator.inactive_since.is_some());
    assert_eq!(
        validator.jail_release,
        Some(jailed_setup.jail_release_block_state.number)
    );
    assert!(!jailed_setup
        .staking_contract
        .active_validators
        .contains_key(&jailed_setup.validator_address));
}
