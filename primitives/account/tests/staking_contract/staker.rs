use nimiq_account::*;
use nimiq_database::{
    traits::{Database, WriteTransaction},
    volatile::VolatileDatabase,
    DatabaseProxy,
};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin, policy::Policy};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::staking_contract::IncomingStakingTransactionData, SignatureProof,
};

use super::*;

fn make_deactivate_stake_transaction(value: u64) -> Transaction {
    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);
    make_signed_incoming_transaction(
        IncomingStakingTransactionData::SetInactiveStake {
            new_inactive_balance: Coin::try_from(value).unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &key_pair,
    )
}

fn make_unstake_transaction(value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        staker_address(),
        AccountType::Basic,
        (value - 100).try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let sig = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()));

    let proof = OutgoingStakingTransactionProof::RemoveStake { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

struct StakerSetup {
    env: DatabaseProxy,
    accounts: Accounts,
    staking_contract: StakingContract,
    before_release_block_state: BlockState,
    inactive_release_block_state: BlockState,
    validator_address: Address,
    staker_address: Address,
    active_stake: Coin,
    inactive_stake: Coin,
    jail_release: Option<u32>,
}

fn setup_staker_with_inactive_balance(
    validator_is_jailed: bool,
    active_stake: u64,
    inactive_stake: u64,
) -> StakerSetup {
    let deactivation_block_state = BlockState::new(2, 2);
    let inactive_release_block_state = BlockState::new(
        Policy::block_after_reporting_window(deactivation_block_state.number),
        2,
    );
    let before_release_block_state = BlockState::new(inactive_release_block_state.number - 1, 2);
    let jail_release = if validator_is_jailed {
        Some(Policy::block_after_jail(deactivation_block_state.number))
    } else {
        None
    };

    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn_og = env.write_transaction();
    let mut db_txn = (&mut db_txn_og).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), false);

    let staker_address = staker_address();
    let active_stake = Coin::from_u64_unchecked(active_stake);
    let inactive_stake = Coin::from_u64_unchecked(inactive_stake);
    let validator_address = validator_address();

    // Create staker.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);
    staking_contract
        .create_staker(
            &mut staking_contract_store,
            &staker_address,
            active_stake + inactive_stake,
            Some(validator_address.clone()),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create staker");

    // Deactivate part of the stake.
    staking_contract
        .set_inactive_stake(
            &mut staking_contract_store,
            &staker_address,
            inactive_stake,
            deactivation_block_state.number,
            &mut TransactionLog::empty(),
        )
        .expect("Failed to set inactive stake");

    if let Some(jail_release) = jail_release {
        let result = staking_contract
            .jail_validator(
                &mut staking_contract_store,
                &validator_address,
                deactivation_block_state.number,
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
    }

    db_txn_og.commit();

    StakerSetup {
        env,
        accounts,
        staking_contract,
        before_release_block_state,
        inactive_release_block_state,
        validator_address,
        staker_address,
        active_stake,
        inactive_stake,
        jail_release,
    }
}

#[test]
fn can_get_it() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address())
        .expect("Staker should exist");

    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
}

#[test]
fn can_iter_stakers() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let stakers =
        staking_contract.get_stakers_for_validator(&data_store.read(&db_txn), &validator_address());

    assert_eq!(stakers.len(), 1);
    assert_eq!(stakers[0].balance, Coin::from_u64_unchecked(150_000_000));
}

#[test]
fn create_staker_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), false);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let validator_address = validator_address();

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
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
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
fn stake_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let validator_address = validator_address();

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
    assert_eq!(staker.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 300_000_000)
    );

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
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );

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
fn update_staker_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let validator_address = validator_address();
    let other_validator_address = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staking_contract
        .create_validator(
            &mut store,
            &other_validator_address,
            signing_key,
            voting_key,
            other_validator_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // Fails before deactivation.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(other_validator_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let result = staking_contract.commit_incoming_transaction(
        &tx,
        &block_state,
        data_store.write(&mut db_txn),
        &mut tx_logger,
    );
    assert_eq!(result, Err(AccountError::InvalidForRecipient));

    // Deactivate stake.
    let deactivate_tx = make_deactivate_stake_transaction(150_000_000);
    let mut tx_logger = TransactionLog::empty();
    let _receipt = staking_contract
        .commit_incoming_transaction(
            &deactivate_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    // Works when changing to another validator.
    let block_state = BlockState::new(Policy::block_after_reporting_window(2), 2);
    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address.clone()),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address.clone()),
            new_validator_address: Some(other_validator_address.clone()),
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker.balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(other_validator_address.clone()));

    let old_validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        old_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(old_validator.num_stakers, 0);

    let new_validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &other_validator_address)
        .expect("Validator should exist");

    assert_eq!(
        new_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(new_validator.num_stakers, 0);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the staker doesn't exist.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(staker_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
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
            proof: SignatureProof::default(),
        },
        0,
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

    let expected_receipt = StakerReceipt {
        delegation: Some(other_validator_address.clone()),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(other_validator_address.clone()),
            new_validator_address: None,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(staker.balance, Coin::ZERO);
    assert_eq!(staker.delegation, None);

    let other_validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &other_validator_address)
        .expect("Validator should exist");

    assert_eq!(
        other_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(other_validator.num_stakers, 0);

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
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
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(other_validator_address.clone()),
            new_validator_address: None,
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &other_validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the staker doesn't exist.
    let fake_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
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
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: (&fake_keypair.public).into()
        })
    );
}

#[test]
fn unstake_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_address = staker_address();
    let validator_address = validator_address();

    // Deactivate stake first.
    let tx = make_deactivate_stake_transaction(150_000_000);
    let _receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Doesn't work if the value is greater than the balance.
    let block_state = BlockState::new(Policy::block_after_reporting_window(2), 2);
    let tx = make_unstake_transaction(200_000_000);

    assert_eq!(
        staking_contract.commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_000),
            balance: Coin::from_u64_unchecked(150_000_000)
        })
    );

    // Works in the valid case.
    let tx = make_unstake_transaction(100_000_000);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RemoveStakeReceipt {
        delegation: Some(validator_address.clone()),
        inactive_release: Some(block_state.number),
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
            Log::Unstake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(100_000_000),
            }
        ]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(50_000_000)
    );
    assert_eq!(staker.balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

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
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Works when removing the entire balance.
    let tx = make_unstake_transaction(50_000_000);

    let block_state = BlockState::new(Policy::block_after_reporting_window(3), 3);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RemoveStakeReceipt {
        delegation: Some(validator_address.clone()),
        inactive_release: Some(block_state.number),
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
            Log::Unstake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(50_000_000),
            }
        ]
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

    // Revert the transaction.
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
            Log::Unstake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(50_000_000),
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

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(50_000_000)
    );
    assert_eq!(staker.balance, Coin::ZERO);
    assert_eq!(staker.delegation, Some(validator_address.clone()));

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
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn unstake_from_tombstone_works() {
    let env = VolatileDatabase::new(20).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_address = staker_address();
    let validator_address = validator_address();

    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);
    staking_contract
        .retire_validator(
            &mut store,
            &validator_address,
            0,
            &mut TransactionLog::empty(),
        )
        .unwrap();
    staking_contract
        .delete_validator(
            &mut store,
            &validator_address,
            Policy::block_after_reporting_window(0) + 1,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .unwrap();

    // Deactivate the stake.
    let deactivate_stake_tx = make_deactivate_stake_transaction(150_000_000);

    let block_state = BlockState::new(Policy::block_after_reporting_window(0) + 2, 1);

    let deactivate_stake_receipt = staking_contract
        .commit_incoming_transaction(
            &deactivate_stake_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let expected_receipt = SetInactiveStakeReceipt {
        old_inactive_release: None,
        old_active_balance: Coin::try_from(150_000_000).unwrap(),
    };
    assert_eq!(deactivate_stake_receipt, Some(expected_receipt.into()));

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    // Remove the staker.
    let unstake_tx = make_unstake_transaction(150_000_000);

    let unstake_block_state = BlockState::new(
        Policy::block_after_reporting_window(block_state.number),
        block_state.time + 1,
    );

    let unstake_receipt = staking_contract
        .commit_outgoing_transaction(
            &unstake_tx,
            &unstake_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RemoveStakeReceipt {
        delegation: Some(validator_address.clone()),
        inactive_release: Some(Policy::block_after_reporting_window(block_state.number)),
    };
    assert_eq!(unstake_receipt, Some(expected_receipt.into()));

    assert_eq!(
        staking_contract.get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    assert_eq!(staking_contract.balance, Coin::ZERO);

    // Revert the unstake transaction.
    staking_contract
        .revert_outgoing_transaction(
            &unstake_tx,
            &unstake_block_state,
            unstake_receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    // Revert the deactivate stake transaction.
    staking_contract
        .revert_incoming_transaction(
            &deactivate_stake_tx,
            &block_state,
            deactivate_stake_receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to revert transaction");

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
}

/// Staker can only unstake inactive balances
#[test]
fn can_only_unstake_inactive_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(false, 50_000_000, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work if the value is greater than the inactive balance.
    let tx = make_unstake_transaction(100_000_000);

    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &staker_setup.inactive_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(100_000_000),
            balance: Coin::from_u64_unchecked(50_000_000)
        })
    );

    // Works if there is enough inactive balance.
    let tx = make_unstake_transaction(50_000_000);

    let mut tx_logger = TransactionLog::empty();
    let _receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &staker_setup.inactive_release_block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot unstake while jailed (although it is already released)
#[test]
fn unstake_jail_interaction() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(true, 50_000_000, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work while validator is jailed.
    let tx = make_unstake_transaction(50_000_000);

    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &staker_setup.inactive_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForSender)
    );

    // Works after jail release.
    let _receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &BlockState::new(staker_setup.jail_release.unwrap(), 1000),
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot redelegate while jailed (although it is already released)
#[test]
fn update_staker_jail_interaction() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(true, 0, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Create second validator.
    let other_validator_address = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staker_setup
        .staking_contract
        .create_validator(
            &mut store,
            &other_validator_address,
            signing_key,
            voting_key,
            other_validator_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // Prepare update transaction.
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(other_validator_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work while validator is jailed.
    assert_eq!(
        staker_setup.staking_contract.commit_incoming_transaction(
            &tx,
            &staker_setup.inactive_release_block_state,
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
            &BlockState::new(staker_setup.jail_release.unwrap(), 1000),
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot unstake before release
#[test]
fn can_only_unstake_after_release() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(false, 50_000_000, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Doesn't work before release.
    let tx = make_unstake_transaction(50_000_000);

    assert_eq!(
        staker_setup.staking_contract.commit_outgoing_transaction(
            &tx,
            &staker_setup.before_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForSender)
    );

    // Works after release.
    let _receipt = staker_setup
        .staking_contract
        .commit_outgoing_transaction(
            &tx,
            &staker_setup.inactive_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot redelegate before release
#[test]
fn can_only_redelegate_after_release() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(false, 0, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Create second validator.
    let other_validator_address = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staker_setup
        .staking_contract
        .create_validator(
            &mut store,
            &other_validator_address,
            signing_key,
            voting_key,
            other_validator_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // Prepare update transaction.
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(other_validator_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

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
            &staker_setup.inactive_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");
}

/// Staker cannot redelegate while having active stake
#[test]
fn cannot_redelegate_while_having_active_stake() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(false, 50_000_000, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // Create second validator.
    let other_validator_address = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staker_setup
        .staking_contract
        .create_validator(
            &mut store,
            &other_validator_address,
            signing_key,
            voting_key,
            other_validator_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // Prepare update transaction.
    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(other_validator_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

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
            &staker_setup.inactive_release_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );
}

/// Updating inactive balance resets counter
#[test]
fn can_update_inactive_balance() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let mut staker_setup = setup_staker_with_inactive_balance(false, 50_000_000, 50_000_000);
    let data_store = staker_setup
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = staker_setup.env.write_transaction();
    let mut db_txn = (&mut db_txn).into();

    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Can update inactive stake.
    let tx = make_deactivate_stake_transaction(100_000_000);

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
            SetInactiveStakeReceipt {
                old_inactive_release: Some(staker_setup.inactive_release_block_state.number),
                old_active_balance: staker_setup.active_stake,
            }
            .into()
        )
    );

    assert_eq!(
        tx_logs.logs,
        vec![Log::SetInactiveStake {
            staker_address: staker_setup.staker_address.clone(),
            validator_address: Some(staker_setup.validator_address.clone()),
            value: Coin::from_u64_unchecked(100_000_000),
            inactive_release: Some(Policy::block_after_reporting_window(
                staker_setup.before_release_block_state.number
            ))
        }]
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.balance, Coin::ZERO);
    assert_eq!(
        staker.inactive_balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(
        staker.inactive_release,
        Some(Policy::block_after_reporting_window(
            staker_setup.before_release_block_state.number
        ))
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

    assert_eq!(staker.balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, staker_setup.inactive_stake);
    assert_eq!(
        staker.inactive_release,
        Some(staker_setup.inactive_release_block_state.number)
    );

    // Can update inactive stake to 0.
    let tx = make_deactivate_stake_transaction(0);

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
            SetInactiveStakeReceipt {
                old_inactive_release: Some(staker_setup.inactive_release_block_state.number),
                old_active_balance: staker_setup.active_stake,
            }
            .into()
        )
    );

    let staker = staker_setup
        .staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_setup.staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(staker.inactive_balance, Coin::ZERO);
    assert_eq!(staker.inactive_release, None);

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

    assert_eq!(staker.balance, staker_setup.active_stake);
    assert_eq!(staker.inactive_balance, staker_setup.inactive_stake);
    assert_eq!(
        staker.inactive_release,
        Some(staker_setup.inactive_release_block_state.number)
    );
}
