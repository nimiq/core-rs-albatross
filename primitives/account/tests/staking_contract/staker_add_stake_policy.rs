use nimiq_account::*;
use nimiq_database::{mdbx::MdbxDatabase, traits::Database};
use nimiq_primitives::{
    account::AccountError, coin::Coin, policy::Policy, transaction::TransactionError,
};
use nimiq_test_log::test;
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;

use super::*;

fn add_stake(protocol_version: u16) {
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2, protocol_version);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let (validator_address, staker_address, mut staking_contract) =
        make_sample_contract_with_protocol_version(
            data_store.write(&mut db_txn),
            Some(150_000_000),
            protocol_version,
        );
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
            receipt,
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

#[test]
fn add_stake_works() {
    for v in 1..=Policy::max_supported_version() {
        add_stake(v);
    }
}

#[test]
fn add_stake_does_not_touch_inactive_or_retired_balances() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance_and_protocol(
        ValidatorState::Active,
        0,
        Policy::MINIMUM_STAKE,
        Policy::MINIMUM_STAKE + 1,
        Policy::max_supported_version(),
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
    // Add stake operation credits to staker.
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
            value: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");
    assert_eq!(
        staker.delegation,
        Some(staker_setup.validator_address.clone())
    );

    assert_eq!(
        staker.active_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker.inactive_from,
        Some(Policy::genesis_block_number() + Policy::blocks_per_epoch())
    );
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE + 1)
    );

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE + Policy::MINIMUM_STAKE * 2 + 1
        )
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
    assert_eq!(
        staker.delegation,
        Some(staker_setup.validator_address.clone())
    );

    assert_eq!(staker.active_balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, staker_setup.inactive_stake);
    assert_eq!(
        staker.inactive_from,
        Some(staker_setup.effective_block_state.number)
    );
    assert_eq!(staker.retired_balance, staker_setup.retired_stake);

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE + Policy::MINIMUM_STAKE + 1
        )
    );
}

#[test]
fn add_stake_enforces_minimum_stake_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance_and_protocol(
        ValidatorState::Active,
        0,
        0,
        50_000_000,
        Policy::max_supported_version(),
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
    // Cannot add less than minimum stake here because it would violate
    // invariant 1 - minimum stake for non-retired funds.
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
            value: Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(
        staker.active_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
    );
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.inactive_from, None);
    assert_eq!(staker.retired_balance, Coin::from_u64_unchecked(50_000_000));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000 + Policy::MINIMUM_STAKE)
    );
}

#[test]
fn add_stake_enforces_greater_than_zero_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance_and_protocol(
        ValidatorState::Active,
        0,
        Policy::MINIMUM_STAKE,
        50_000_000,
        Policy::max_supported_version(),
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
    // Cannot add zero stake.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_setup.staker_address.clone(),
        },
        0,
        &staker_keypair,
    );
    assert_eq!(
        tx.verify(NetworkId::UnitAlbatross, 1),
        Err(TransactionError::ZeroValue)
    );

    // Can add in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_setup.staker_address.clone(),
        },
        1,
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
            value: Coin::from_u64_unchecked(1),
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(1),);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker.inactive_from,
        Some(staker_setup.effective_block_state.number)
    );
    assert_eq!(staker.retired_balance, Coin::from_u64_unchecked(50_000_000));

    let validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 1)
    );
    assert_eq!(
        staker_setup.staking_contract.balance,
        Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 50_000_000 + Policy::MINIMUM_STAKE + 1
        )
    );
}
