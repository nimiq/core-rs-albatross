use rand::{rngs::StdRng, SeedableRng};
use std::convert::TryFrom;
use std::time::Instant;
use tempfile::tempdir;

use nimiq_account::{Accounts, Inherent, InherentType};
use nimiq_account::{Receipt, Receipts};
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_database::WriteTransaction;
use nimiq_database::{mdbx::MdbxEnvironment, volatile::VolatileEnvironment};
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair, PublicKey, SecureGenerate};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_test_log::test;
use nimiq_test_utils::test_transaction::{
    generate_accounts, generate_transactions, TestTransaction,
};
use nimiq_transaction::Transaction;
use nimiq_trie::key_nibbles::KeyNibbles;

const VOLATILE_ENV: bool = true;

#[test]
fn it_can_commit_and_revert_a_block_body() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator = Address::from([1u8; Address::SIZE]);

    let address_recipient = Address::from([2u8; Address::SIZE]);

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let mut receipts = vec![Receipt::Inherent {
        index: 0,
        pre_transactions: false,
        data: None,
    }];

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_validator), None),
        None
    );

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.commit(&mut txn, &[], &[reward.clone()], 1, 1),
        Ok(Receipts::from(receipts.clone()))
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    let hash1 = accounts.get_root(None);

    let tx = Transaction::new_basic(
        address_validator.clone(),
        address_recipient.clone(),
        Coin::from_u64_unchecked(10),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let transactions = vec![tx];

    receipts.insert(
        0,
        Receipt::Transaction {
            index: 0,
            sender: false,
            data: None,
        },
    );

    receipts.insert(
        0,
        Receipt::Transaction {
            index: 0,
            sender: true,
            data: None,
        },
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.commit(&mut txn, &transactions, &[reward.clone()], 2, 2),
        Ok(Receipts::from(receipts.clone()))
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10)
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000 + 10000 - 10)
    );

    assert_ne!(hash1, accounts.get_root(None));

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.revert(
            &mut txn,
            &transactions,
            &[reward],
            2,
            2,
            &Receipts::from(receipts)
        ),
        Ok(())
    );

    txn.commit();

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(hash1, accounts.get_root(None));
}

#[test]
fn it_correctly_rewards_validators() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator_1 = Address::from([1u8; Address::SIZE]);

    let address_validator_2 = Address::from([2u8; Address::SIZE]);

    let address_recipient_1 = Address::from([3u8; Address::SIZE]);

    let address_recipient_2 = Address::from([4u8; Address::SIZE]);

    // Validator 1 mines first block.
    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_validator_1), None),
        None
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_1.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts.commit(&mut txn, &[], &[reward], 1, 1).is_ok());

    txn.commit();

    // Create transactions to Recipient 1 and Recipient 2.
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_1), None)
            .unwrap()
            .balance(),
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
        accounts.get(&KeyNibbles::from(&address_validator_2), None),
        None
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_2.clone(),
        value: Coin::from_u64_unchecked(10000) + fee1 + fee2,
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &vec![tx1, tx2], &[reward], 2, 2)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_1), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000) - value1 - fee1 - value2 - fee2
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_2), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000) + fee1 + fee2
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient_1), None)
            .unwrap()
            .balance(),
        value1
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient_2), None)
            .unwrap()
            .balance(),
        value2
    );
}

#[test]
fn it_checks_for_sufficient_funds() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_sender = Address::from([1u8; Address::SIZE]);

    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut tx = Transaction::new_basic(
        address_sender.clone(),
        address_recipient.clone(),
        Coin::try_from(10).unwrap(),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_sender.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let hash1 = accounts.get_root(None);

    assert_eq!(accounts.get(&KeyNibbles::from(&address_sender), None), None);

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    // Fails as address_sender does not have any funds.
    // Note: When the commit errors, we want to bracket the txn creation and the commit attempt.
    // Otherwise when we try to commit again, the test will get stuck.
    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 1, 1)
            .is_err());
    }

    assert_eq!(accounts.get(&KeyNibbles::from(&address_sender), None), None);

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(hash1, accounts.get_root(None));

    // Give address_sender one block reward.

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &[], &[reward.clone()], 1, 1)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    let hash2 = accounts.get_root(None);

    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = Coin::from_u64_unchecked(1000000);

    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 2, 2)
            .is_err());
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(hash2, accounts.get_root(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from_u64_unchecked(5010);

    let mut tx2 = tx.clone();

    tx2.value += Coin::from_u64_unchecked(10);

    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &vec![tx, tx2], &[reward], 2, 2)
            .is_err());
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts.get(&KeyNibbles::from(&address_recipient), None),
        None
    );

    assert_eq!(hash2, accounts.get_root(None));
}

#[test]
fn accounts_performance() {
    let (env, num_txns) = if VOLATILE_ENV {
        let num_txns = 1_000;
        let env = VolatileEnvironment::new(10).unwrap();

        (env, num_txns)
    } else {
        let num_txns = 10_000;
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        log::debug!("Creating a non volatile environment in {}", tmp_dir);
        let env = MdbxEnvironment::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
        (env, num_txns)
    };

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);
    let balance = 100;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![num_txns as u64 * 10; num_txns];
    let recipient_balances = vec![0; num_txns];
    let mut genesis_builder = GenesisBuilder::default();
    let address_validator = Address::from([1u8; Address::SIZE]);
    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };
    let rewards = vec![reward; num_txns];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, false);
    log::debug!("Done generating {} transactions and accounts", txns.len());

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        PublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = WriteTransaction::new(&env);
    let start = Instant::now();
    accounts.init(&mut txn, genesis_info.accounts);
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

    let mut txn = WriteTransaction::new(&env);
    let start = Instant::now();
    let result = accounts.commit(&mut txn, &txns[..], &rewards[..], 1, 1);
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
        "Time ellapsed after txn commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
}

#[test]
fn accounts_performance_history_sync_batches_single_sender() {
    let num_batches = 5;

    let (env, num_txns) = if VOLATILE_ENV {
        let num_txns = 25;
        let env = VolatileEnvironment::new(10).unwrap();

        (env, num_txns)
    } else {
        let num_txns = 100;
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        log::debug!("Creating a non volatile environment in {}", tmp_dir);
        let env = MdbxEnvironment::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
        (env, num_txns)
    };

    let total_txns = num_batches * policy::BLOCKS_PER_BATCH * num_txns;

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);

    let sender_balances = vec![100_000_000_000; 1];
    let recipient_balances = vec![0; total_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();
    let rewards = vec![];

    // Generate accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    let total_blocks = num_batches * policy::BLOCKS_PER_BATCH;

    let mut block_transactions = vec![];

    let mut txn_index = 0;

    for _block in 0..total_blocks {
        // Generate a new set of txns for each block
        let mut mempool_transactions = vec![];
        for _i in 0..num_txns {
            let mempool_transaction = TestTransaction {
                fee: 0 as u64,
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
        PublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = WriteTransaction::new(&env);
    let start = Instant::now();
    accounts.init(&mut txn, genesis_info.accounts);
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
        let mut txn = WriteTransaction::new(&env);

        let batch_start = Instant::now();

        for _blocks in 0..policy::BLOCKS_PER_BATCH {
            let result = accounts.commit_batch(
                &mut txn,
                &block_transactions[block_index as usize],
                &rewards[..],
                block_index,
                1,
            );
            match result {
                Ok(_) => assert!(true),
                Err(err) => assert!(false, "Received {}", err),
            };
            block_index += 1;
        }

        accounts.finalize_batch(&mut txn);
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
        let env = VolatileEnvironment::new(10).unwrap();

        (env, num_txns)
    } else {
        let num_txns = 100;
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        log::debug!("Creating a non volatile environment in {}", tmp_dir);
        let env = MdbxEnvironment::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
        (env, num_txns)
    };

    let total_txns = num_batches * policy::BLOCKS_PER_BATCH * num_txns;

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);

    let sender_balances = vec![100_000_000_000; total_txns as usize];
    let recipient_balances = vec![10; total_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();
    let rewards = vec![];

    // Generate accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, true);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    let total_blocks = num_batches * policy::BLOCKS_PER_BATCH;

    let mut block_transactions = vec![];

    let mut txn_index = 0;

    for _block in 0..total_blocks {
        // Generate transactions
        let mut mempool_transactions = vec![];
        for _i in 0..num_txns {
            let mempool_transaction = TestTransaction {
                fee: 0 as u64,
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
        PublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = WriteTransaction::new(&env);
    let start = Instant::now();
    accounts.init(&mut txn, genesis_info.accounts);
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
        let mut txn = WriteTransaction::new(&env);

        let batch_start = Instant::now();

        for _blocks in 0..policy::BLOCKS_PER_BATCH {
            let result = accounts.commit_batch(
                &mut txn,
                &block_transactions[block_index as usize],
                &rewards[..],
                block_index,
                1,
            );
            match result {
                Ok(_) => assert!(true),
                Err(err) => assert!(false, "Received {}", err),
            };
            block_index += 1;
        }

        accounts.finalize_batch(&mut txn);
        txn.commit();

        let batch_duration = batch_start.elapsed();

        log::debug!(
            "Processed batch {}, duration {}ms",
            batch,
            batch_duration.as_millis(),
        );
    }
}
