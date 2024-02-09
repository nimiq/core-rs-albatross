use nimiq_account::*;
use nimiq_database::{traits::Database, volatile::VolatileDatabase};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin, policy::Policy};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
    SignatureProof,
};
use nimiq_trie::WriteTransactionProxy;

use super::*;

fn make_retire_stake_transaction(value: u64) -> Transaction {
    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);
    make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireStake {
            retire_stake: Coin::try_from(value).unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &key_pair,
    )
}

fn make_activate_stake_transaction(value: u64) -> Transaction {
    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);
    make_signed_incoming_transaction(
        IncomingStakingTransactionData::SetActiveStake {
            new_active_balance: Coin::try_from(value).unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &key_pair,
    )
}

fn make_remove_stake_transaction(value: u64) -> Transaction {
    make_remove_stake_transaction_with_fee(value - 100, 100)
}

fn make_remove_stake_transaction_with_fee(value: u64, fee: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        OutgoingStakingTransactionData::RemoveStake.serialize_to_vec(),
        staker_address(),
        AccountType::Basic,
        vec![],
        value.try_into().unwrap(),
        fee.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);
    let signature = key_pair.sign(&tx.serialize_content());

    tx.proof = SignatureProof::from(key_pair.public, signature).serialize_to_vec();

    tx
}

fn prepare_second_validator_for_redelegation(
    validator_state: ValidatorState,
    active_stake: u64,
    inactive_stake: u64,
    retired_stake: u64,
) -> (StakerSetup, Address, Transaction) {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        validator_state,
        active_stake,
        inactive_stake,
        retired_stake,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn_og = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn_og).into();

    // Create second validator.
    let validator_address2 = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staker_setup
        .staking_contract
        .create_validator(
            &mut store,
            &validator_address2,
            signing_key,
            voting_key,
            validator_address2.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            None,
            None,
            false,
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // Prepare update transaction.
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(validator_address2.clone()),
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    db_txn_og.commit();

    (staker_setup, validator_address2, tx)
}

#[test]
fn can_get_staker() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (_, staker_address, staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), Some(150_000_000));

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address.unwrap())
        .expect("Staker should exist");

    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(150_000_000));
}

#[test]
fn can_iter_stakers() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), Some(150_000_000));

    let stakers =
        staking_contract.get_stakers_for_validator(&data_store.read(&db_txn), &validator_address);

    assert_eq!(stakers.len(), 1);
    assert_eq!(
        stakers[0].active_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
}

#[test]
fn create_staker_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, _, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), None);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(validator_address.clone()),
            proof: SignatureProof::default(),
        },
        150_000_000,
        &staker_keypair,
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
        vec![Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Doesn't work when the staker already exists.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::AlreadyExistentAddress {
            address: staker_address.clone()
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
        vec![Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    assert_eq!(
        staking_contract.get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn add_stake_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, staker_address, mut staking_contract) =
        make_sample_contract(data_store.write(&mut db_txn), Some(150_000_000));
    let staker_address = staker_address.unwrap();
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_address.clone(),
        },
        150_000_000,
        &staker_keypair,
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
        vec![Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 300_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 300_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 300_000_000
        ))
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
        vec![Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );
}

/// Adding stake cannot violate minimum stake for non-retired balances.
#[test]
fn add_stake_enforces_minimum_stake() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        0,
        0,
        50_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot add less than minimum stake.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_setup.staker_address.clone(),
        },
        Policy::MINIMUM_STAKE - 1,
        &staker_keypair,
    );

    let mut tx_logs = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut tx_logs,
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Can add in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_setup.staker_address.clone(),
        },
        Policy::MINIMUM_STAKE,
        &staker_keypair,
    );

    let mut tx_logs = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut tx_logs,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);

    assert_eq!(
        tx_logs.logs,
        vec![Log::Stake {
            staker_address: staker_setup.staker_address.clone(),
            validator_address: Some(staker_setup.validator_address.clone()),
            value: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(
        staker.active_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.inactive_from, None);

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE)
    );
}

/// Updating inactive balance resets counter
#[test]
fn can_set_inactive_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        50_000_000,
        50_000_000,
        10_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Can update inactive stake.
    let tx = make_activate_stake_transaction(0);

    let mut tx_logs = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut tx_logs,
        )
        .expect("Failed to commit transaction");

    assert_eq!(
        receipt,
        Some(
            SetActiveStakeReceipt {
                old_inactive_from: Some(staker_setup.effective_block_state.number),
                old_active_balance: staker_setup.active_stake,
            }
            .into()
        )
    );

    assert_eq!(
        tx_logs.logs,
        vec![Log::SetActiveStake {
            staker_address: staker_setup.staker_address.clone(),
            validator_address: Some(staker_setup.validator_address.clone()),
            active_balance: Coin::ZERO,
            inactive_balance: Coin::from_u64_unchecked(100_000_000),
            inactive_from: Some(Policy::election_block_after(
                staker_setup.before_release_block_state.number
            ))
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(
        staker.inactive_from,
        Some(Policy::election_block_after(
            staker_setup.before_release_block_state.number
        ))
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    // Reverts correctly.
    staker_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, staker_setup.inactive_stake);
    assert_eq!(
        staker.inactive_from,
        Some(staker_setup.effective_block_state.number)
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(50_000_000 + Policy::VALIDATOR_DEPOSIT)
    );

    // Can update inactive stake to 0.
    let tx = make_activate_stake_transaction(100_000_000);

    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    assert_eq!(
        receipt,
        Some(
            SetActiveStakeReceipt {
                old_inactive_from: Some(staker_setup.effective_block_state.number),
                old_active_balance: staker_setup.active_stake,
            }
            .into()
        )
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.inactive_from, None);

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(100_000_000 + Policy::VALIDATOR_DEPOSIT)
    );

    // Reverts correctly.
    staker_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, staker_setup.inactive_stake);
    assert_eq!(
        staker.inactive_from,
        Some(staker_setup.effective_block_state.number)
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(50_000_000 + Policy::VALIDATOR_DEPOSIT)
    );
}

#[test]
fn retire_inactive_stake_works_with_min_active_stake() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        Policy::MINIMUM_STAKE,
        Policy::MINIMUM_STAKE + 1,
        1,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let validator_address = staker_setup.validator_address;
    let validator_deposit =
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT) + staker_setup.active_stake;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot retire if not enough funds
    let tx = make_retire_stake_transaction(Policy::MINIMUM_STAKE + 2);
    let block_state = staker_setup.release_block_state.clone();

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 2),
            balance: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 1)
        })
    );

    // Can partially retire stake.
    let tx_1 = make_retire_stake_transaction(Policy::MINIMUM_STAKE);

    let mut tx_logger = TransactionLog::empty();
    let receipt_1 = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx_1,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireStakeReceipt {
        old_inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt_1, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::RetireStake {
            validator_address: Some(validator_address.clone()),
            staker_address: staker_setup.staker_address.clone(),
            inactive_balance: Coin::from_u64_unchecked(1),
            retired_balance: staker_setup.retired_stake
                + Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
            inactive_from: Some(staker_setup.effective_block_state.number),
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(
            &data_store.read(&db_txn),
            &staker_setup.staker_address.clone(),
        )
        .expect("Staker should exist");

    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 1)
    );
    assert_eq!(
        staker.inactive_from,
        Some(staker_setup.effective_block_state.number)
    );
    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, Coin::from_u64_unchecked(1));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    // Validator should have the same counter as before.
    let validator_after_update = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator_after_update.total_stake, validator_deposit);
    assert_eq!(validator_after_update.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&validator_deposit)
    );

    // Retires the remainder
    let tx_2 = make_retire_stake_transaction(1);

    let mut tx_logger = TransactionLog::empty();
    let receipt_2 = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx_2,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireStakeReceipt {
        old_inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt_2, Some(expected_receipt.into()));

    let staker = staker_setup
        .staking_contract
        .get_staker(
            &data_store.read(&db_txn),
            &staker_setup.staker_address.clone(),
        )
        .expect("Staker should exist");
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 2)
    );
    assert_eq!(staker.inactive_from, None);
    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    // Revert the transactions.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx_2,
            &block_state,
            receipt_2,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::RetireStake {
            validator_address: Some(validator_address.clone()),
            staker_address: staker_setup.staker_address.clone(),
            inactive_balance: Coin::ZERO,
            retired_balance: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 2),
            inactive_from: None,
        }]
    );

    let mut tx_logger = TransactionLog::empty();
    staker_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx_1,
            &block_state,
            receipt_1,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::RetireStake {
            validator_address: Some(validator_address.clone()),
            staker_address: staker_setup.staker_address.clone(),
            inactive_balance: Coin::from_u64_unchecked(1),
            retired_balance: staker_setup.retired_stake
                + Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
            inactive_from: Some(staker_setup.effective_block_state.number),
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(
            &data_store.read(&db_txn),
            &staker_setup.staker_address.clone(),
        )
        .expect("Staker should exist");

    assert_eq!(staker.retired_balance, Coin::from_u64_unchecked(1));
    assert_eq!(
        staker.inactive_from,
        Some(staker_setup.effective_block_state.number)
    );
    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 1)
    );
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator_after_revert = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator_after_revert.total_stake, validator_deposit);
    assert_eq!(validator_after_revert.num_stakers, 1);
    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&validator_deposit)
    );
}

/// Staker cannot retire stake while jailed (although it is already released)
#[test]
fn retire_stake_does_not_violate_jail_or_inactive_releases() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_balance(
        ValidatorState::Active,
        0,
        Policy::MINIMUM_STAKE,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot retire before cooldown.
    let tx = make_retire_stake_transaction(Policy::MINIMUM_STAKE);
    let block_state = staker_setup.before_release_block_state;

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Cannot retire before jail.

    // Setup jailed validator
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_balance(
        ValidatorState::Jailed,
        0,
        Policy::MINIMUM_STAKE,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let block_state = BlockState {
        number: staker_setup.validator_state_release.unwrap() - 1,
        time: 1,
    };

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Works after all releases

    let tx_1 = make_retire_stake_transaction(Policy::MINIMUM_STAKE);
    let mut tx_logger = TransactionLog::empty();

    let receipt_1 = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx_1,
            &BlockState {
                number: staker_setup.validator_state_release.unwrap(),
                time: 1,
            },
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireStakeReceipt {
        old_inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt_1, Some(expected_receipt.into()));
}

#[test]
fn retire_stake_does_not_violate_minimum_active_stake() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        0,
        Policy::MINIMUM_STAKE + 1,
        1,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let validator_address = staker_setup.validator_address;
    let validator_deposit =
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT) + staker_setup.active_stake;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot retire stake if less than minimum active+inactive stake.
    let tx = make_retire_stake_transaction(Policy::MINIMUM_STAKE);
    let block_state = staker_setup.release_block_state.clone();

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Works if it retires everything

    let tx_1 = make_retire_stake_transaction(Policy::MINIMUM_STAKE + 1);
    let mut tx_logger = TransactionLog::empty();

    let receipt_1 = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx_1,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireStakeReceipt {
        old_inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt_1, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::RetireStake {
            validator_address: Some(validator_address.clone()),
            staker_address: staker_setup.staker_address.clone(),
            inactive_balance: Coin::ZERO,
            retired_balance: staker_setup.retired_stake
                + Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 1),
            inactive_from: None,
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(
            &data_store.read(&db_txn),
            &staker_setup.staker_address.clone(),
        )
        .expect("Staker should exist");

    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 2)
    );
    assert_eq!(staker.inactive_from, None);
    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    // Validator should have the same counter as before.
    let validator_after_update = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator_after_update.total_stake, validator_deposit);
    assert_eq!(validator_after_update.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&validator_deposit)
    );
}

#[test]
fn cannot_retire_active_stake() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        Policy::MINIMUM_STAKE,
        0,
        1,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Cannot retire funds from active.
    let tx = make_retire_stake_transaction(Policy::MINIMUM_STAKE);
    let block_state = BlockState::new(
        Policy::block_after_reporting_window(Policy::election_block_after(2)),
        2,
    );
    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
            balance: Coin::ZERO
        })
    );
}

#[test]
fn update_staker_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, validator_address2, tx) =
        prepare_second_validator_for_redelegation(ValidatorState::Active, 0, 150_000_000, 0);

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address.clone();
    let validator_address1 = staker_setup.validator_address.clone();
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works when changing to another validator.
    let block_state = staker_setup.release_block_state;

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    let mut tx_logger = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address1.clone()),
        active_balance: staker.active_balance,
        inactive_from: staker.inactive_from,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address1.clone()),
            new_validator_address: Some(validator_address2.clone()),
            active_balance: staker.active_balance,
            inactive_from: staker.inactive_from,
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address2.clone()));

    let old_validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        old_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(old_validator.num_stakers, 0);

    let new_validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        new_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(new_validator.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the staker doesn't exist.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(staker_address.clone()),
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: staker_address.clone()
        })
    );

    // Works when changing to no validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    let mut tx_logger = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address2.clone()),
        active_balance: staker.active_balance,
        inactive_from: staker.inactive_from,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address2.clone()),
            new_validator_address: None,
            active_balance: staker.active_balance,
            inactive_from: staker.inactive_from,
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.delegation, None);

    let other_validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        other_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(other_validator.num_stakers, 0);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
        .staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address2.clone()),
            new_validator_address: None,
            active_balance: staker.active_balance,
            inactive_from: staker.inactive_from,
        }]
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the staker doesn't exist.
    let fake_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &fake_keypair,
    );

    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: (&fake_keypair.public).into()
        })
    );
}

#[test]
fn update_staker_with_stake_reactivation_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup =
        StakerSetup::setup_staker_with_inactive_balance(ValidatorState::Active, 0, 150_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let mut data_store_write = data_store.write(&mut db_txn);

    let staker_address = staker_setup.staker_address;
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    let validator_address1 = staker_setup.validator_address;
    let validator_address2 = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);
    staker_setup
        .staking_contract
        .create_validator(
            &mut store,
            &validator_address2,
            signing_key,
            voting_key,
            validator_address2.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            None,
            None,
            false,
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works when changing to another validator.
    let block_state = staker_setup.release_block_state;
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(validator_address2.clone()),
            reactivate_all_stake: true,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address1.clone()),
        active_balance: Coin::ZERO,
        inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    let staker_after = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address1.clone()),
            new_validator_address: Some(validator_address2.clone()),
            active_balance: staker_after.active_balance,
            inactive_from: staker_after.inactive_from,
        }]
    );

    assert!(staker_after.inactive_from.is_none());
    assert_eq!(staker_after.address, staker_address);
    assert_eq!(staker_after.inactive_balance, Coin::ZERO);
    assert_eq!(
        staker_after.active_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker_after.delegation, Some(validator_address2.clone()));

    let validator1 = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        validator1.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator1.num_stakers, 0);

    let validator2 = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        validator2.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT) + staker_after.active_balance
    );
    assert_eq!(validator2.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&(Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT) + staker_after.active_balance))
    );

    // Revert it
    let mut tx_logger = TransactionLog::empty();
    staker_setup
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
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address1.clone()),
            new_validator_address: Some(validator_address2.clone()),
            active_balance: staker_after.active_balance,
            inactive_from: staker_after.inactive_from,
        }]
    );

    let validator2 = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        validator2.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator2.num_stakers, 0);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    let validator1 = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        validator1.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator1.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn update_staker_remove_delegation_with_stake_reactivation_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup =
        StakerSetup::setup_staker_with_inactive_balance(ValidatorState::Active, 0, 150_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address;
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let validator_address1 = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works when changing to no validator.
    let block_state = staker_setup.release_block_state;
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            reactivate_all_stake: true,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address1.clone()),
        active_balance: Coin::ZERO,
        inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    let staker_after = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address1.clone()),
            new_validator_address: None,
            active_balance: staker_after.active_balance,
            inactive_from: staker_after.inactive_from,
        }]
    );

    assert_eq!(staker_after.address, staker_address);
    assert_eq!(staker_after.inactive_balance, Coin::ZERO);
    assert_eq!(
        staker_after.active_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker_after.delegation, None);
    assert!(staker_after.inactive_from.is_none());

    let validator2 = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        validator2.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator2.num_stakers, 0);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
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
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address1.clone()),
            new_validator_address: None,
            active_balance: staker_after.active_balance,
            inactive_from: staker_after.inactive_from,
        }]
    );

    let validator1 = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        validator1.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator1.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn update_staker_with_no_delegation_no_reactivation() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    // Create a validator with no staker
    let mut validator_setup = ValidatorSetup::new(None);
    let data_store = validator_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = validator_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();
    let validator_address1 = validator_setup.validator_address;

    // Create a staker with no delegation
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let block_state = BlockState::new(2, 2);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: None,
            proof: SignatureProof::default(),
        },
        150_000_000,
        &staker_keypair,
    );

    let receipt = validator_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works when changing to no validator.
    let block_state = BlockState::new(3, 3);
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(validator_address1.clone()),
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
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

    let expected_receipt = StakerReceipt {
        delegation: None,
        active_balance: Coin::from_u64_unchecked(150_000_000),
        inactive_from: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    let staker_after = validator_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: None,
            new_validator_address: Some(validator_address1.clone()),
            active_balance: staker_after.active_balance,
            inactive_from: staker_after.inactive_from,
        }]
    );

    assert_eq!(staker_after.address, staker_address);
    assert_eq!(staker_after.inactive_balance, Coin::ZERO);
    assert_eq!(
        staker_after.active_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker_after.delegation, Some(validator_address1.clone()));
    assert!(staker_after.inactive_from.is_none());

    let validator1 = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        validator1.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator1.num_stakers, 1);

    assert_eq!(
        validator_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
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
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: None,
            new_validator_address: Some(validator_address1.clone()),
            active_balance: staker_after.active_balance,
            inactive_from: staker_after.inactive_from,
        }]
    );

    let validator1 = validator_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address1)
        .expect("Validator should exist");

    assert_eq!(
        validator1.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator1.num_stakers, 0);

    assert_eq!(
        validator_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn update_staker_same_validator() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup =
        StakerSetup::setup_staker_with_inactive_balance(ValidatorState::Active, 0, 150_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address;
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let validator_address = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works when changing to no validator.
    let block_state = staker_setup.release_block_state.clone();
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(validator_address.clone()),
            reactivate_all_stake: false,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    // Checks validator before update operation has the right counter
    let validator_before_update = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator_before_update.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator_before_update.num_stakers, 1);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address.clone()),
        active_balance: Coin::ZERO,
        inactive_from: Some(staker_setup.effective_block_state.number),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address.clone()),
            new_validator_address: Some(validator_address.clone()),
            active_balance: Coin::ZERO,
            inactive_from: Some(staker_setup.effective_block_state.number),
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    // Validator should have the same counter as before.
    let validator_after_update = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator_after_update.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator_after_update.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
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
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address.clone()),
            new_validator_address: Some(validator_address.clone()),
            active_balance: Coin::ZERO,
            inactive_from: Some(staker_setup.effective_block_state.number),
        }]
    );

    let validator_after_revert = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator_after_revert.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator_after_revert.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn remove_stake_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        0,
        0,
        Policy::MINIMUM_STAKE * 2,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address;
    let validator_address = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work if the value is greater than the balance.
    let block_state = BlockState::new(
        Policy::block_after_reporting_window(Policy::election_block_after(2)),
        2,
    );
    let tx = make_remove_stake_transaction(Policy::MINIMUM_STAKE * 2 + 1);

    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Partial remove stake does not work.
    let tx = make_remove_stake_transaction(Policy::MINIMUM_STAKE);

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Works when removing the entire balance.
    let tx = make_remove_stake_transaction(Policy::MINIMUM_STAKE * 2);

    let block_state = BlockState::new(
        Policy::block_after_reporting_window(Policy::election_block_after(2)),
        3,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = DeleteStakerReceipt {
        delegation: Some(validator_address.clone()),
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
            Log::RemoveStake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE * 2),
            },
            Log::DeleteStaker {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone())
            },
        ]
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
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
            Log::DeleteStaker {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone())
            },
            Log::RemoveStake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE * 2),
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

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE * 2)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE * 2)
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn remove_stake_from_tombstone_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Deleted,
        0,
        0,
        150_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address;
    let validator_address = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Remove the staker.
    let remove_stake_tx = make_remove_stake_transaction(150_000_000);
    let remove_stake_block_state = staker_setup.release_block_state;

    let remove_stake_receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &remove_stake_tx,
            &remove_stake_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let expected_receipt = DeleteStakerReceipt {
        delegation: Some(validator_address.clone()),
    };
    assert_eq!(remove_stake_receipt, Some(expected_receipt.into()));

    assert_eq!(
        staker_setup
            .staking_contract
            .get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );
    assert_eq!(
        staker_setup
            .staking_contract
            .get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(staker_setup.staking_contract.balance, Coin::ZERO);

    // Revert the remove stake transaction.
    staker_setup
        .staking_contract
        .revert_outgoing_transaction(
            &remove_stake_tx,
            &remove_stake_block_state,
            remove_stake_receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        staker_setup
            .staking_contract
            .get_tombstone(&data_store.read(&db_txn), &validator_address),
        Some(Tombstone {
            remaining_stake: Coin::ZERO,
            num_remaining_stakers: 1
        })
    );
}

/// Staker can only remove stake from retired balance
#[test]
fn can_only_remove_retired_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        50_000_000,
        50_000_000,
        50_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Assert the num of stakers of the validator
    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(
        validator.total_stake,
        staker_setup.active_stake + Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 1);

    // Doesn't work if the value is greater than the retired balance.
    let tx = make_remove_stake_transaction(100_000_000);

    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Doesn't work if we are not withdrawing all funds.
    let tx = make_remove_stake_transaction(50_000_000 - 1);
    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Works if there is enough retired balance.
    let tx = make_remove_stake_transaction(50_000_000);

    let mut tx_logger = TransactionLog::empty();
    let _receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        staker_setup.active_stake + Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    // Doesn't work if not enough funds.
    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidCoinValue)
    );
}

#[test]
fn can_remove_stake_with_no_delegation() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, validator_address2, tx) = prepare_second_validator_for_redelegation(
        ValidatorState::Jailed,
        0,
        100_000_000,
        50_000_000,
    );

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Update delegation
    let _receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &BlockState::new(staker_setup.validator_state_release.unwrap(), 2),
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");
    assert_eq!(staker.delegation, Some(validator_address2.clone()));
    assert_eq!(staker.retired_balance, Coin::from_u64_unchecked(50_000_000));

    // Removes stake.
    let tx = make_remove_stake_transaction(50_000_000);
    let receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");
    assert_eq!(staker.delegation, Some(validator_address2.clone()));
    assert_eq!(staker.retired_balance, Coin::ZERO);
}

/// Staker cannot re delegate while jailed although inactive funds are already released.
#[test]
fn can_only_redelegate_after_jail() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, _validator_address2, tx) =
        prepare_second_validator_for_redelegation(ValidatorState::Jailed, 0, 50_000_000, 0);

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work while validator is jailed.
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Works after jail release.
    let _receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &BlockState::new(staker_setup.validator_state_release.unwrap(), 1000),
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot re delegate before inactive release.
#[test]
fn can_only_redelegate_after_release() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, _validator_address2, tx) =
        prepare_second_validator_for_redelegation(ValidatorState::Active, 0, 50_000_000, 0);

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work before release.
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Works after release.
    let _receipt = staker_setup
        .staking_contract
        .commit_incoming_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot re delegate while having active stake
#[test]
fn cannot_redelegate_while_having_active_stake() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, _validator_address2, tx) = prepare_second_validator_for_redelegation(
        ValidatorState::Active,
        50_000_000,
        50_000_000,
        0,
    );

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work before release.
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Doesn't work after release.
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );
}

#[test]
fn can_reserve_and_release_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        40_000_000,
        10_000_000,
        60_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let _write = data_store.write(&mut db_txn);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Reserve balance for partial remove stake does not work.
    let mut reserved_balance = ReservedBalance::new(staker_setup.staker_address.clone());

    let tx = make_remove_stake_transaction(50_000_000);
    let result = staker_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &staker_setup.retire_stake_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::ZERO);
    assert_eq!(result, Err(AccountError::InvalidCoinValue));

    // Reserves the total retired stake.
    let tx = make_remove_stake_transaction(60_000_000);
    let result = staker_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &staker_setup.retire_stake_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(60_000_000)
    );
    assert!(result.is_ok());

    // Cannot reserve balance for further remove stake transactions.
    let tx = make_remove_stake_transaction(10_000_000);
    let result = staker_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &staker_setup.retire_stake_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(60_000_000)
    );
    assert_eq!(result, Err(AccountError::InvalidCoinValue));

    // Can release balance.
    let tx = make_remove_stake_transaction(60_000_000);
    let result = staker_setup.staking_contract.release_balance(
        &tx,
        &mut reserved_balance,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::ZERO);
    assert!(result.is_ok());

    // Can reserve balance for remove stake again.
    let tx = make_remove_stake_transaction(60_000_000);
    let result = staker_setup.staking_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &staker_setup.retire_stake_block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(
        reserved_balance.balance(),
        Coin::from_u64_unchecked(60_000_000)
    );
    assert!(result.is_ok());
}

#[test]
fn commit_failed_remove_stake_works_when_removing_staker() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        0,
        0,
        Policy::MINIMUM_STAKE + 1,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address;
    let validator_address = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work if the fee is greater than the balance.
    let block_state = staker_setup.retire_stake_block_state;

    let tx = make_remove_stake_transaction_with_fee(0, Policy::MINIMUM_STAKE + 2);

    assert_eq!(
        staker_setup.staking_contract.commit_failed_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 2),
            balance: staker_setup.retired_stake
        })
    );

    // Partial remove stake.

    // Does not work if fee deduction would violates min stake
    let tx_1 = make_remove_stake_transaction_with_fee(0, Policy::MINIMUM_STAKE);
    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_failed_transaction(
            &tx_1,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::InvalidCoinValue)
    );

    // Works in the valid case.
    let tx_1 = make_remove_stake_transaction_with_fee(0, 1);

    let mut tx_logger = TransactionLog::empty();
    let receipt_1 = staker_setup
        .staking_contract
        .commit_failed_transaction(
            &tx_1,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt_1, None);
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::pay_fee_log(&tx_1),
            Log::StakerFeeDeduction {
                staker_address: staker_address.clone(),
                fee: tx_1.fee,
            }
        ]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");
    assert_eq!(validator.num_stakers, 1);

    // Works when removing the entire balance.
    let tx_2 = make_remove_stake_transaction_with_fee(0, Policy::MINIMUM_STAKE);

    let mut tx_logger = TransactionLog::empty();
    let receipt_2 = staker_setup
        .staking_contract
        .commit_failed_transaction(
            &tx_2,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = DeleteStakerReceipt {
        delegation: Some(validator_address.clone()),
    };
    assert_eq!(receipt_2, Some(expected_receipt.into()));
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::pay_fee_log(&tx_2),
            Log::StakerFeeDeduction {
                staker_address: staker_address.clone(),
                fee: tx_2.fee,
            },
            Log::DeleteStaker {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone())
            },
        ]
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
        .staking_contract
        .revert_failed_transaction(
            &tx_2,
            &block_state,
            receipt_2,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::DeleteStaker {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone())
            },
            Log::StakerFeeDeduction {
                staker_address: staker_address.clone(),
                fee: tx_2.fee,
            },
            Log::pay_fee_log(&tx_2)
        ]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE)
    );
}

#[test]
fn commit_failed_remove_stake_works_with_non_retired_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Active,
        0,
        Policy::MINIMUM_STAKE,
        Policy::MINIMUM_STAKE,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address;
    let validator_address = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the valid case.
    let block_state = staker_setup.retire_stake_block_state;

    // Works when removing the entire balance.
    let tx_2 = make_remove_stake_transaction_with_fee(0, Policy::MINIMUM_STAKE);

    let mut tx_logger = TransactionLog::empty();
    let receipt_2 = staker_setup
        .staking_contract
        .commit_failed_transaction(
            &tx_2,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt_2, None);
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::pay_fee_log(&tx_2),
            Log::StakerFeeDeduction {
                staker_address: staker_address.clone(),
                fee: tx_2.fee,
            }
        ]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.retired_balance, Coin::ZERO);
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staker_setup
        .staking_contract
        .revert_failed_transaction(
            &tx_2,
            &block_state,
            receipt_2,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::StakerFeeDeduction {
                staker_address: staker_address.clone(),
                fee: tx_2.fee,
            },
            Log::pay_fee_log(&tx_2)
        ]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE + Policy::MINIMUM_STAKE
        )
    );
}
