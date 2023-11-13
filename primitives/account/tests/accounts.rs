use std::{convert::TryFrom, time::Instant};

use log::info;
use nimiq_account::{
    Account, Accounts, BasicAccount, BlockLogger, BlockState, InherentOperationReceipt, Log,
    OperationReceipt, TransactionOperationReceipt, TransactionReceipt, VestingContract,
};
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
    volatile::VolatileDatabase,
};
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, EdDSAPublicKey, KeyPair, PrivateKey, SecureGenerate};
use nimiq_primitives::{
    account::{AccountType, FailReason},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{JailedValidator, PenalizedSlot},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{
    accounts_revert::TestCommitRevert,
    test_rng::test_rng,
    test_transaction::{generate_accounts, generate_transactions, TestTransaction},
    transactions::{IncomingType, OutgoingType, TransactionsGenerator, ValidatorState},
};
use nimiq_transaction::{inherent::Inherent, EdDSASignatureProof, SignatureProof, Transaction};
use rand::Rng;
use tempfile::tempdir;

const VOLATILE_ENV: bool = true;

#[test]
fn it_can_commit_and_revert_a_block_body() {
    let accounts = TestCommitRevert::new();

    let address_validator = Address::from([1u8; Address::SIZE]);
    let address_recipient = Address::from([2u8; Address::SIZE]);

    let reward = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: address_validator.clone(),
        value: Coin::from_u64_unchecked(10000),
    };

    let inherent_receipts = vec![InherentOperationReceipt::Ok(None)];

    assert_eq!(
        accounts.get_complete(&address_validator, None),
        Account::default(),
    );

    let block_state = BlockState::new(1, 1);

    let receipts = accounts
        .commit_and_test(
            &[],
            &[reward.clone()],
            &block_state,
            &mut BlockLogger::empty(),
        )
        .unwrap();

    assert_eq!(receipts.inherents, inherent_receipts);
    assert_eq!(receipts.transactions, vec![]);

    assert_eq!(
        accounts.get_complete(&address_validator, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    let hash1 = accounts.get_root_hash_assert(None);

    let tx = Transaction::new_basic(
        address_validator.clone(),
        address_recipient.clone(),
        Coin::from_u64_unchecked(10),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let transactions = vec![tx];

    let transaction_receipts = vec![TransactionOperationReceipt::Ok(
        TransactionReceipt::default(),
    )];

    assert_eq!(
        accounts.get_complete(&address_recipient, None),
        Account::default()
    );

    let block_state = BlockState::new(2, 2);

    let receipts = accounts
        .commit_and_test(
            &transactions,
            &[reward.clone()],
            &block_state,
            &mut BlockLogger::empty(),
        )
        .unwrap();

    assert_eq!(receipts.inherents, inherent_receipts);
    assert_eq!(receipts.transactions, transaction_receipts);

    assert_eq!(
        accounts.get_complete(&address_recipient, None).balance(),
        Coin::from_u64_unchecked(10)
    );

    assert_eq!(
        accounts.get_complete(&address_validator, None).balance(),
        Coin::from_u64_unchecked(10000 + 10000 - 10)
    );

    assert_ne!(hash1, accounts.get_root_hash_assert(None));

    let mut txn = accounts.env.write_transaction();

    accounts
        .revert(
            &mut (&mut txn).into(),
            &transactions,
            &[reward],
            &block_state,
            receipts.into(),
            &mut BlockLogger::empty(),
        )
        .unwrap();

    txn.commit();

    assert_eq!(
        accounts.get_complete(&address_recipient, None),
        Account::default(),
    );

    assert_eq!(
        accounts.get_complete(&address_validator, None).balance(),
        Coin::from_u64_unchecked(10000),
    );

    assert_eq!(hash1, accounts.get_root_hash_assert(None));
}

#[test]
fn it_correctly_rewards_validators() {
    let accounts = TestCommitRevert::new();

    let address_validator_1 = Address::from([1u8; Address::SIZE]);
    let address_validator_2 = Address::from([2u8; Address::SIZE]);
    let address_recipient_1 = Address::from([3u8; Address::SIZE]);
    let address_recipient_2 = Address::from([4u8; Address::SIZE]);

    // Validator 1 mines first block.
    assert_eq!(
        accounts.get_complete(&address_validator_1, None),
        Account::default(),
    );

    let reward = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: address_validator_1.clone(),
        value: Coin::from_u64_unchecked(10000),
    };

    let block_state = BlockState::new(1, 1);

    assert!(accounts
        .commit_and_test(&[], &[reward], &block_state, &mut BlockLogger::empty())
        .is_ok());

    // Create transactions to Recipient 1 and Recipient 2.
    assert_eq!(
        accounts.get_complete(&address_validator_1, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    let value1 = Coin::from_u64_unchecked(5);
    let fee1 = Coin::from_u64_unchecked(3);
    let value2 = Coin::from_u64_unchecked(7);
    let fee2 = Coin::from_u64_unchecked(11);

    let tx1 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_1.clone(),
        value1,
        fee1,
        2,
        NetworkId::Main,
    );

    let tx2 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_2.clone(),
        value2,
        fee2,
        2,
        NetworkId::Main,
    );

    // Validator 2 mines second block.
    assert_eq!(
        accounts.get_complete(&address_validator_2, None),
        Account::default()
    );

    let reward = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: address_validator_2.clone(),
        value: Coin::from_u64_unchecked(10000) + fee1 + fee2,
    };

    let block_state = BlockState::new(2, 2);

    assert!(accounts
        .commit_and_test(
            &[tx1, tx2],
            &[reward],
            &block_state,
            &mut BlockLogger::empty()
        )
        .is_ok());

    assert_eq!(
        accounts.get_complete(&address_validator_1, None).balance(),
        Coin::from_u64_unchecked(10000) - value1 - fee1 - value2 - fee2
    );

    assert_eq!(
        accounts.get_complete(&address_validator_2, None).balance(),
        Coin::from_u64_unchecked(10000) + fee1 + fee2
    );

    assert_eq!(
        accounts.get_complete(&address_recipient_1, None).balance(),
        value1
    );

    assert_eq!(
        accounts.get_complete(&address_recipient_2, None).balance(),
        value2
    );
}

#[test]
fn it_checks_for_sufficient_funds() {
    let accounts = TestCommitRevert::new();

    let address_sender = Address::from([1u8; Address::SIZE]);
    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut tx = Transaction::new_basic(
        address_sender.clone(),
        address_recipient.clone(),
        Coin::try_from(10).unwrap(),
        Coin::from_u64_unchecked(1),
        1,
        NetworkId::Main,
    );

    let reward = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: address_sender.clone(),
        value: Coin::from_u64_unchecked(10000),
    };

    let hash1 = accounts.get_root_hash_assert(None);

    assert_eq!(
        accounts.get_complete(&address_sender, None),
        Account::default()
    );

    assert_eq!(
        accounts.get_complete(&address_recipient, None),
        Account::default()
    );

    // Fails as sender address does not exist.
    // Note this kind of transaction would be rejected by the mempool.
    let block_state = BlockState::new(1, 1);
    assert!(accounts
        .commit_and_test(
            &[tx.clone()],
            &[reward.clone()],
            &block_state,
            &mut BlockLogger::empty()
        )
        .is_err());

    assert_eq!(
        accounts.get_complete(&address_sender, None),
        Account::default(),
    );

    assert_eq!(
        accounts.get_complete(&address_recipient, None),
        Account::default()
    );

    assert_eq!(hash1, accounts.get_root_hash_assert(None));

    // Give address_sender one block reward.
    assert!(accounts
        .commit_and_test(&[], &[reward], &block_state, &mut BlockLogger::empty())
        .is_ok());

    assert_eq!(
        accounts.get_complete(&address_sender, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get_complete(&address_recipient, None),
        Account::default()
    );

    let hash2 = accounts.get_root_hash_assert(None);

    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    // FIXME Adjust commit behaviour to fail entirely if the funds are insufficient?
    //  The mempool should reject such transactions, therefore they should not be allowed in a block.
    tx.value = Coin::from_u64_unchecked(1000000);
    let block_state = BlockState::new(2, 2);
    {
        let mut txn = accounts.env.write_transaction();

        let receipts = accounts
            .commit(
                &mut (&mut txn).into(),
                &[tx.clone()],
                &[],
                &block_state,
                &mut BlockLogger::empty(),
            )
            .unwrap();

        assert_eq!(
            receipts.transactions,
            vec![TransactionOperationReceipt::Err(
                TransactionReceipt::default(),
                FailReason::InsufficientFunds
            )]
        );
    }

    assert_eq!(
        accounts.get_complete(&address_sender, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get_complete(&address_recipient, None),
        Account::default()
    );

    assert_eq!(hash2, accounts.get_root_hash_assert(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from_u64_unchecked(5010);
    let mut tx2 = tx.clone();
    tx2.value += Coin::from_u64_unchecked(10);

    let receipts = accounts
        .commit_and_test(&vec![tx, tx2], &[], &block_state, &mut BlockLogger::empty())
        .unwrap();

    assert_eq!(
        receipts.transactions,
        vec![
            TransactionOperationReceipt::Ok(TransactionReceipt::default()),
            TransactionOperationReceipt::Err(
                TransactionReceipt::default(),
                FailReason::InsufficientFunds
            ),
        ]
    );

    assert_eq!(
        accounts.get_complete(&address_sender, None).balance(),
        Coin::from_u64_unchecked(4988)
    );

    assert_eq!(
        accounts.get_complete(&address_recipient, None).balance(),
        Coin::from_u64_unchecked(5010)
    );
}

#[test]
fn accounts_performance() {
    let (env, num_txns) = if VOLATILE_ENV {
        let num_txns = 1_000;
        let env = VolatileDatabase::new(20).unwrap();

        (env, num_txns)
    } else {
        let num_txns = 10_000;
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        log::debug!("Creating a non volatile environment in {}", tmp_dir);
        let env = MdbxDatabase::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
        (env, num_txns)
    };

    // Generate and sign transaction from an address
    let mut rng = test_rng(true);
    let balance = 100;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![num_txns as u64 * 10; num_txns];
    let recipient_balances = vec![0; num_txns];
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    let address_validator = Address::from([1u8; Address::SIZE]);
    let reward = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: address_validator,
        value: Coin::from_u64_unchecked(10000),
    };
    let rewards = vec![reward; num_txns];

    // Generate recipient accounts
    let recipient_accounts =
        generate_accounts(recipient_balances, &mut genesis_builder, false, &mut rng);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true, &mut rng);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance,
            recipient: recipient_accounts[i].clone(),
            sender: sender_accounts[i].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, false);
    log::debug!("Done generating {} transactions and accounts", txns.len());

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        EdDSAPublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = env.write_transaction();
    let start = Instant::now();
    accounts.init(&mut (&mut txn).into(), genesis_info.accounts);
    let duration = start.elapsed();
    println!(
        "Time elapsed after account init: {} ms, Accounts per second {}",
        duration.as_millis(),
        length as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
    let start = Instant::now();
    txn.commit();
    let duration = start.elapsed();
    println!(
        "Time elapsed after account init's txn commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );

    println!("Done adding accounts to genesis {}", txns.len());

    let mut txn = env.write_transaction();
    let block_state = BlockState::new(1, 1);
    let start = Instant::now();
    let result = accounts.commit(
        &mut (&mut txn).into(),
        &txns[..],
        &rewards[..],
        &block_state,
        &mut BlockLogger::empty(),
    );
    match result {
        Ok(_) => assert!(true),
        Err(err) => assert!(false, "Received {}", err),
    };
    let duration = start.elapsed();
    println!(
        "Time elapsed after account commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
    let start = Instant::now();
    txn.commit();
    let duration = start.elapsed();
    println!(
        "Time elapsed after txn commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
}

#[test]
fn accounts_performance_history_sync_batches_single_sender() {
    let num_batches = 5;

    let (env, num_txns) = if VOLATILE_ENV {
        let num_txns = 25;
        let env = VolatileDatabase::new(20).unwrap();

        (env, num_txns)
    } else {
        let num_txns = 100;
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        log::debug!("Creating a non volatile environment in {}", tmp_dir);
        let env = MdbxDatabase::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
        (env, num_txns)
    };

    let total_txns = num_batches * Policy::blocks_per_batch() * num_txns;

    // Generate and sign transaction from an address
    let mut rng = test_rng(true);

    let sender_balances = vec![100_000_000_000; 1];
    let recipient_balances = vec![0; total_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    let rewards = vec![];

    // Generate accounts
    let recipient_accounts =
        generate_accounts(recipient_balances, &mut genesis_builder, false, &mut rng);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true, &mut rng);

    let total_blocks = num_batches * Policy::blocks_per_batch();

    let mut block_transactions = vec![];

    let mut txn_index = 0;

    for _block in 0..total_blocks {
        // Generate a new set of txns for each block
        let mut mempool_transactions = vec![];
        for _i in 0..num_txns {
            let mempool_transaction = TestTransaction {
                fee: 0_u64,
                value: 1,
                recipient: recipient_accounts[txn_index as usize].clone(),
                sender: sender_accounts[0].clone(),
            };
            mempool_transactions.push(mempool_transaction);
            txn_index += 1;
        }
        let (txns, _) = generate_transactions(mempool_transactions.clone(), false);
        block_transactions.push(txns);
    }

    log::debug!("Done generating {} transactions and accounts", txn_index);

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        EdDSAPublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = env.write_transaction();
    let start = Instant::now();
    accounts.init(&mut (&mut txn).into(), genesis_info.accounts);
    let duration = start.elapsed();
    log::debug!(
        "Time elapsed after account init: {} ms, Accounts per second {}",
        duration.as_millis(),
        length as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
    let start = Instant::now();
    txn.commit();
    let duration = start.elapsed();
    log::debug!(
        "Time elapsed after account init's txn commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );

    log::debug!("Done adding accounts to genesis");

    let mut block_index = 0;

    for batch in 0..num_batches {
        let mut txn = env.write_transaction();

        let batch_start = Instant::now();

        for _blocks in 0..Policy::blocks_per_batch() {
            let block_state = BlockState::new(block_index, 1);
            let result = accounts.commit_batch(
                &mut (&mut txn).into(),
                &block_transactions[block_index as usize],
                &rewards[..],
                &block_state,
                &mut BlockLogger::empty(),
            );
            match result {
                Ok(_) => assert!(true),
                Err(err) => assert!(false, "Received {}", err),
            };
            block_index += 1;
        }

        accounts.finalize_batch(&mut (&mut txn).into());
        txn.commit();

        let batch_duration = batch_start.elapsed();

        log::debug!(
            "Processed batch {}, duration {}ms",
            batch,
            batch_duration.as_millis()
        );
    }
}

#[test]
fn accounts_performance_history_sync_batches_many_to_many() {
    let num_batches = 5;

    let (env, num_txns) = if VOLATILE_ENV {
        let num_txns = 25;
        let env = VolatileDatabase::new(20).unwrap();

        (env, num_txns)
    } else {
        let num_txns = 100;
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        log::debug!("Creating a non volatile environment in {}", tmp_dir);
        let env = MdbxDatabase::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
        (env, num_txns)
    };

    let total_txns = num_batches * Policy::blocks_per_batch() * num_txns;

    // Generate and sign transaction from an address
    let mut rng = test_rng(true);

    let sender_balances = vec![100_000_000_000; total_txns as usize];
    let recipient_balances = vec![10; total_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    let rewards = vec![];

    // Generate accounts
    let recipient_accounts =
        generate_accounts(recipient_balances, &mut genesis_builder, true, &mut rng);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true, &mut rng);

    let total_blocks = num_batches * Policy::blocks_per_batch();

    let mut block_transactions = vec![];

    let mut txn_index = 0;

    for _block in 0..total_blocks {
        // Generate transactions
        let mut mempool_transactions = vec![];
        for _i in 0..num_txns {
            let mempool_transaction = TestTransaction {
                fee: 0_u64,
                value: 1,
                recipient: recipient_accounts[txn_index as usize].clone(),
                sender: sender_accounts[txn_index as usize].clone(),
            };
            mempool_transactions.push(mempool_transaction);
            txn_index += 1;
        }
        let (txns, _) = generate_transactions(mempool_transactions.clone(), false);
        block_transactions.push(txns);
    }

    log::debug!("Done generating {} transactions and accounts", txn_index);

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        EdDSAPublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = env.write_transaction();
    let start = Instant::now();
    accounts.init(&mut (&mut txn).into(), genesis_info.accounts);
    let duration = start.elapsed();
    log::debug!(
        "Time elapsed after account init: {} ms, Accounts per second {}",
        duration.as_millis(),
        length as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
    let start = Instant::now();
    txn.commit();
    let duration = start.elapsed();
    log::debug!(
        "Time elapsed after account init's txn commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );

    let mut block_index = 0;

    for batch in 0..num_batches {
        let mut txn = env.write_transaction();

        let batch_start = Instant::now();

        for _blocks in 0..Policy::blocks_per_batch() {
            let block_state = BlockState::new(block_index, 1);
            let result = accounts.commit_batch(
                &mut (&mut txn).into(),
                &block_transactions[block_index as usize],
                &rewards[..],
                &block_state,
                &mut BlockLogger::empty(),
            );
            match result {
                Ok(_) => assert!(true),
                Err(err) => assert!(false, "Received {}", err),
            };
            block_index += 1;
        }

        accounts.finalize_batch(&mut (&mut txn).into());
        txn.commit();

        let batch_duration = batch_start.elapsed();

        log::debug!(
            "Processed batch {}, duration {}ms",
            batch,
            batch_duration.as_millis(),
        );
    }
}

#[test]
fn it_commits_valid_and_failing_txns() {
    let priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();

    let key_pair = KeyPair::from(priv_key);

    // We need to populate the accounts trie with different testing accounts
    // Vesting Contract
    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    let accounts = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_pair.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(1000),
            }),
        ),
        (Address([1u8; 20]), Account::Vesting(start_contract)),
    ]);

    // Create a transaction that tries to move more funds than available
    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        2000.try_into().unwrap(),
        200.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.sender_type = AccountType::Vesting;

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof =
        SignatureProof::EdDSA(EdDSASignatureProof::from(key_pair.public, signature));
    tx.proof = signature_proof.serialize_to_vec();

    let block_state = BlockState::new(1, 200);
    let receipts = accounts
        .commit_and_test(&[tx], &[], &block_state, &mut BlockLogger::empty())
        .unwrap();

    assert_eq!(
        receipts.transactions,
        vec![TransactionOperationReceipt::Err(
            TransactionReceipt::default(),
            FailReason::InsufficientFunds
        )]
    );

    // We should deduct the fee from the vesting contract balance
    assert_eq!(
        accounts
            .get_complete(&Address::from([1u8; 20]), None)
            .balance(),
        Coin::from_u64_unchecked(800)
    );

    // The fee should not be deducted from the sender
    assert_eq!(
        accounts
            .get_complete(&Address::from(&key_pair.public), None)
            .balance(),
        Coin::from_u64_unchecked(1000)
    );

    // Now send a transaction from the basic account that should fail (value exceeds the account balance but fee can be paid)
    let mut tx = Transaction::new_basic(
        Address::from(&key_pair.public),
        Address::from([2u8; 20]),
        2000.try_into().unwrap(),
        200.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    // tx.sender_type = AccountType::Basic;

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = EdDSASignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let mut block_logger = BlockLogger::empty();
    let receipts = accounts
        .commit_and_test(&[tx.clone()], &[], &block_state, &mut block_logger)
        .unwrap();
    let block_logs = block_logger.build(0);

    assert_eq!(
        receipts.transactions,
        vec![TransactionOperationReceipt::Err(
            TransactionReceipt::default(),
            FailReason::InsufficientFunds
        )]
    );
    assert!(block_logs.transaction_logs()[0].failed);
    assert_eq!(
        block_logs.transaction_logs()[0].logs,
        vec![
            Log::FailedTransaction {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                failure_reason: FailReason::InsufficientFunds
            },
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee
            }
        ]
    );

    // The fee should be deducted from the sender
    assert_eq!(
        accounts
            .get_complete(&Address::from(&key_pair.public), None)
            .balance(),
        Coin::from_u64_unchecked(800)
    );
}

#[test]
fn can_revert_transactions() {
    let accounts = TestCommitRevert::new();
    let mut generator = TransactionsGenerator::new(
        Accounts::new(accounts.env.clone()),
        NetworkId::UnitAlbatross,
        test_rng(false),
    );

    let block_state = BlockState::new(
        Policy::block_after_reporting_window(Policy::election_block_after(0)),
        10,
    );

    for sender in [
        OutgoingType::Basic,
        OutgoingType::Vesting,
        OutgoingType::HTLCRegularTransfer,
        OutgoingType::HTLCEarlyResolve,
        OutgoingType::HTLCTimeoutResolve,
        OutgoingType::DeleteValidator,
        OutgoingType::RemoveStake,
    ] {
        for recipient in [
            IncomingType::Basic,
            IncomingType::CreateVesting,
            IncomingType::CreateHTLC,
            IncomingType::CreateValidator,
            IncomingType::UpdateValidator,
            IncomingType::DeactivateValidator,
            IncomingType::ReactivateValidator,
            IncomingType::RetireValidator,
            IncomingType::CreateStaker,
            IncomingType::AddStake,
            IncomingType::UpdateStaker,
            IncomingType::SetActiveStake,
        ] {
            // Don't send from the staking contract to the staking contract.
            if matches!(
                sender,
                OutgoingType::DeleteValidator | OutgoingType::RemoveStake
            ) && matches!(
                recipient,
                IncomingType::CreateValidator
                    | IncomingType::UpdateValidator
                    | IncomingType::DeactivateValidator
                    | IncomingType::ReactivateValidator
                    | IncomingType::RetireValidator
                    | IncomingType::CreateStaker
                    | IncomingType::AddStake
                    | IncomingType::UpdateStaker
                    | IncomingType::SetActiveStake
            ) {
                continue;
            }
            for (fail_sender, fail_recipient) in
                [(true, true), (true, false), (false, true), (false, false)]
            {
                if fail_recipient
                    && !recipient.is_staker_related()
                    && !recipient.is_validator_related()
                {
                    continue;
                }
                if fail_sender
                    && matches!(
                        recipient,
                        IncomingType::UpdateValidator
                            | IncomingType::RetireValidator
                            | IncomingType::DeactivateValidator
                            | IncomingType::ReactivateValidator
                            | IncomingType::UpdateStaker
                            | IncomingType::SetActiveStake
                    )
                {
                    continue;
                }
                if let Some(tx) = generator.create_failing_transaction(
                    sender,
                    recipient,
                    Coin::from_u64_unchecked(10),
                    Coin::from_u64_unchecked(1),
                    &block_state,
                    fail_sender,
                    fail_recipient,
                ) {
                    info!(
                        ?sender,
                        ?recipient,
                        fail_sender,
                        fail_recipient,
                        "Testing transaction"
                    );
                    assert_eq!(tx.verify(NetworkId::UnitAlbatross), Ok(()));

                    let receipts = accounts.test(&[tx], &[], &block_state);
                    if fail_sender || fail_recipient {
                        assert!(matches!(
                            receipts.transactions[..],
                            [OperationReceipt::Err(..)]
                        ));
                    } else {
                        assert!(matches!(
                            receipts.transactions[..],
                            [OperationReceipt::Ok(_)]
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn can_revert_inherents() {
    let accounts = TestCommitRevert::new();
    let mut generator = TransactionsGenerator::new(
        Accounts::new(accounts.env.clone()),
        NetworkId::UnitAlbatross,
        test_rng(false),
    );

    let block_state = BlockState::new(
        Policy::blocks_per_epoch() + Policy::blocks_per_batch() + 1,
        10,
    );

    let mut rng = test_rng(false);

    info!("Testing inherent Reward");
    let inherent = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: Address(rng.gen()),
        value: Coin::from_u64_unchecked(10),
    };

    // Inherents must always succeed.
    let receipts = accounts.test(&[], &[inherent], &block_state);
    assert!(matches!(receipts.inherents[..], [OperationReceipt::Ok(_)]));

    let (validator_key_pair, _, _) =
        generator.create_validator_and_staker(ValidatorState::Active, false, false);

    info!("Testing inherent Penalize");
    let inherent = Inherent::Penalize {
        slot: PenalizedSlot {
            slot: rng.gen_range(0..Policy::SLOTS),
            validator_address: Address::from(&validator_key_pair),
            offense_event_block: block_state.number - 1,
        },
    };

    // Inherents must always succeed.
    let receipts = accounts.test(&[], &[inherent], &block_state);
    assert!(matches!(receipts.inherents[..], [OperationReceipt::Ok(_)]));

    info!("Testing inherent Jail");
    let slot_start = rng.gen_range(0..Policy::SLOTS);
    let slot_end = rng.gen_range(slot_start..Policy::SLOTS);
    let inherent = Inherent::Jail {
        jailed_validator: JailedValidator {
            slots: slot_start..slot_end,
            validator_address: Address::from(&validator_key_pair),
            // The `offense_event_block` identifies the block at which the malicious behaviour occurred.
            offense_event_block: block_state.number - 1,
        },
        new_epoch_slot_range: None,
    };

    // Inherents must always succeed.
    let receipts = accounts.test(&[], &[inherent], &block_state);
    assert!(matches!(receipts.inherents[..], [OperationReceipt::Ok(_)]));

    // Testing failing inherent.
    info!("Testing inherent Reward");
    let inherent = Inherent::Reward {
        validator_address: Address::burn_address(),
        target: Policy::STAKING_CONTRACT_ADDRESS,
        value: Coin::from_u64_unchecked(10),
    };

    // Inherents must always succeed.
    let receipts = accounts.test(&[], &[inherent], &block_state);
    assert!(matches!(
        receipts.inherents[..],
        [OperationReceipt::Err(..)]
    ));
}
