use nimiq_account::*;
use nimiq_database::traits::Database;
use nimiq_primitives::{account::AccountError, coin::Coin, policy::Policy};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::staking_contract::IncomingStakingTransactionData, SignatureProof,
};
use nimiq_trie::WriteTransactionProxy;

use super::*;
use crate::staking_contract::staker::*;

#[test]
fn create_staker_with_tombstone_delegation_does_not_work() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Deleted,
        50_000_000,
        50_000_000,
        10_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let tombstone_address = staker_setup.validator_address;
    let staker_keypair = ed25519_key_pair(NON_EXISTENT_PRIVATE_KEY);

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Does not work with tombstone.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(tombstone_address.clone()),
            proof: SignatureProof::default(),
        },
        150_000_000,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::NonExistentAddress {
            address: tombstone_address
        })
    );
}

#[test]
fn add_stake_to_tombstone_does_not_work() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Deleted,
        1,
        Policy::MINIMUM_STAKE,
        50_000_000,
    );

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let initial_staking_contract_balance = staker_setup.staking_contract.balance;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Add stake operation fails due to non-existent validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_setup.staker_address.clone(),
        },
        Policy::MINIMUM_STAKE,
        &staker_keypair,
    );

    let mut tx_logs = TransactionLog::empty();
    let receipt = staker_setup.staking_contract.commit_incoming_transaction(
        &tx,
        &staker_setup.before_release_block_state,
        data_store.write(&mut db_txn),
        &mut tx_logs,
    );
    assert_eq!(
        Err(AccountError::NonExistentAddress {
            address: staker_setup.validator_address.clone()
        }),
        receipt
    );

    assert_eq!(tx_logs.logs, vec![]);

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(
        staker.delegation,
        Some(staker_setup.validator_address.clone())
    );
    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(1));
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker.inactive_from,
        Some(Policy::genesis_block_number() + Policy::blocks_per_epoch())
    );
    assert_eq!(staker.retired_balance, Coin::from_u64_unchecked(50_000_000));

    let validator = staker_setup
        .staking_contract
        .get_tombstone(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_remaining_stakers, 1);
    assert_eq!(validator.remaining_stake, Coin::from_u64_unchecked(1));
    assert_eq!(
        staker_setup.staking_contract.balance,
        initial_staking_contract_balance
    );
}

#[test]
fn add_stake_to_tombstone_does_not_legacy_work_0v() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance_and_protocol(
        ValidatorState::Deleted,
        1,
        Policy::MINIMUM_STAKE,
        50_000_000,
        0,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let initial_staking_contract_balance = staker_setup.staking_contract.balance;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Add stake operation fails due to non-existent validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_setup.staker_address.clone(),
        },
        Policy::MINIMUM_STAKE,
        &staker_keypair,
    );

    let mut tx_logs = TransactionLog::empty();
    let receipt = staker_setup.staking_contract.commit_incoming_transaction(
        &tx,
        &staker_setup.before_release_block_state,
        data_store.write(&mut db_txn),
        &mut tx_logs,
    );
    assert_eq!(
        Err(AccountError::NonExistentAddress {
            address: staker_setup.validator_address.clone()
        }),
        receipt
    );

    assert_eq!(tx_logs.logs, vec![]);

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(
        staker.delegation,
        Some(staker_setup.validator_address.clone())
    );
    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(1));
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
    );
    assert_eq!(
        staker.inactive_from,
        Some(Policy::genesis_block_number() + Policy::blocks_per_epoch())
    );
    assert_eq!(staker.retired_balance, Coin::from_u64_unchecked(50_000_000));

    let validator = staker_setup
        .staking_contract
        .get_tombstone(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_remaining_stakers, 1);
    assert_eq!(validator.remaining_stake, Coin::from_u64_unchecked(1));
    assert_eq!(
        staker_setup.staking_contract.balance,
        initial_staking_contract_balance
    );
}

#[test]
fn can_set_inactive_stake_with_tombstone_delegations() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Deleted,
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
        .get_tombstone(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();

    assert_eq!(validator.num_remaining_stakers, 1);
    assert_eq!(validator.remaining_stake, Coin::ZERO);

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
        .get_tombstone(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_remaining_stakers, 1);
    assert_eq!(
        validator.remaining_stake,
        Coin::from_u64_unchecked(50_000_000)
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
        .get_tombstone(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_remaining_stakers, 1);
    assert_eq!(
        validator.remaining_stake,
        Coin::from_u64_unchecked(100_000_000)
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
        .get_tombstone(&data_store.read(&db_txn), &staker_setup.validator_address)
        .unwrap();
    assert_eq!(validator.num_remaining_stakers, 1);
    assert_eq!(
        validator.remaining_stake,
        Coin::from_u64_unchecked(50_000_000)
    );
}

#[test]
fn redelegate_from_tombstone_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, validator_address2, tx) = prepare_second_validator_for_redelegation(
        ValidatorState::Deleted,
        0,
        150_000_000,
        100_000_000,
        false,
    );

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
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(staker.delegation, Some(validator_address2.clone()));

    let old_validator = staker_setup
        .staking_contract
        .get_tombstone(&data_store.read(&db_txn), &validator_address1);

    assert_eq!(old_validator, None);

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
        None
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Works when reverting.
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
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(staker.delegation, Some(validator_address1.clone()));

    let old_validator = staker_setup
        .staking_contract
        .get_tombstone(&data_store.read(&db_txn), &validator_address1);

    assert_eq!(
        old_validator,
        Some(Tombstone {
            remaining_stake: Coin::ZERO,
            num_remaining_stakers: 1
        })
    );

    let new_validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        new_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(new_validator.num_stakers, 0);

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
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(staker.active_balance, Coin::ZERO);
    assert_eq!(staker.delegation, None);

    assert_eq!(
        staker_setup
            .staking_contract
            .get_tombstone(&data_store.read(&db_txn), &validator_address2),
        None
    );
}

#[test]
fn redelegate_from_tombstone_with_reactivation_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (mut staker_setup, validator_address2, tx) = prepare_second_validator_for_redelegation(
        ValidatorState::Deleted,
        0,
        150_000_000,
        100_000_000,
        true,
    );

    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_address = staker_setup.staker_address.clone();
    let validator_address1 = staker_setup.validator_address.clone();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works when changing to another validator and automatically reactivating stake.
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
            active_balance: Coin::from_u64_unchecked(150_000_000),
            inactive_from: None,
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.active_balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(staker.delegation, Some(validator_address2.clone()));

    let old_validator = staker_setup
        .staking_contract
        .get_tombstone(&data_store.read(&db_txn), &validator_address1);

    assert_eq!(old_validator, None);

    let new_validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        new_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(new_validator.num_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address1),
        None
    );

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address2),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Works when reverting.
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
            active_balance: Coin::from_u64_unchecked(150_000_000),
            inactive_from: None,
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
    assert_eq!(
        staker.retired_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(staker.delegation, Some(validator_address1.clone()));

    let old_validator = staker_setup
        .staking_contract
        .get_tombstone(&data_store.read(&db_txn), &validator_address1);

    assert_eq!(
        old_validator,
        Some(Tombstone {
            remaining_stake: Coin::ZERO,
            num_remaining_stakers: 1
        })
    );

    let new_validator = staker_setup
        .staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address2)
        .expect("Validator should exist");

    assert_eq!(
        new_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(new_validator.num_stakers, 0);
}

#[test]
fn update_staker_with_tombstone_does_not_work() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Deleted,
        0,
        150_000_000,
        100_000_000,
    );
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn: WriteTransactionProxy = (&mut db_txn).into();

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let validator_address = staker_setup.validator_address;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Does not work with tombstone validator.
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
        .get_tombstone(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator_before_update.remaining_stake, Coin::ZERO);
    assert_eq!(validator_before_update.num_remaining_stakers, 1);

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        ),
        Err(AccountError::NonExistentAddress {
            address: validator_address
        })
    );
}

#[test]
fn retire_inactive_stake_from_tombstone_delegation() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = StakerSetup::setup_staker_with_inactive_retired_balance(
        ValidatorState::Deleted,
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
    let validator_initial_remaining_stake = staker_setup.active_stake;

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Can partially retire stake.
    let block_state = staker_setup.release_block_state.clone();
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
        .get_tombstone(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator_after_update.remaining_stake,
        validator_initial_remaining_stake
    );
    assert_eq!(validator_after_update.num_remaining_stakers, 1);

    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        None
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
        .get_tombstone(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator_after_revert.remaining_stake,
        validator_initial_remaining_stake
    );
    assert_eq!(validator_after_revert.num_remaining_stakers, 1);
    assert_eq!(
        staker_setup
            .staking_contract
            .active_validators
            .get(&validator_address),
        None
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
