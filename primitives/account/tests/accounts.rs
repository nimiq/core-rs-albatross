use beserial::{Deserialize, Serialize};
use nimiq_hash::Hash;
use nimiq_primitives::account::AccountType;
use rand::{rngs::StdRng, SeedableRng};
use std::convert::TryFrom;
use std::time::Instant;
use tempfile::tempdir;

use nimiq_account::{
    Account, Accounts, BasicAccount, BatchInfo, Inherent, InherentType, Log, TransactionLog,
    VestingContract,
};
use nimiq_account::{Receipt, Receipts};
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_database::WriteTransaction;
use nimiq_database::{mdbx::MdbxEnvironment, volatile::VolatileEnvironment};
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey, SecureGenerate};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::test_transaction::{
    generate_accounts, generate_transactions, TestTransaction,
};
use nimiq_transaction::{ExecutedTransaction, SignatureProof, Transaction};
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

    let mut tx_logs = Vec::new();

    let inherent_logs = vec![Log::PayoutReward {
        to: reward.target.clone(),
        value: reward.value,
    }];

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap(),
        None
    );

    let mut txn = WriteTransaction::new(&env);

    let (batch_info, _) = accounts
        .commit(&mut txn, &[], &[reward.clone()], 1, 1)
        .unwrap();

    assert_eq!(
        batch_info,
        BatchInfo::new(receipts.clone(), tx_logs.clone(), inherent_logs.clone())
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .unwrap()
            .balance(),
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

    let transactions = vec![tx.clone()];

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

    tx_logs.push(TransactionLog::new(
        tx.hash(),
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
        ],
    ));

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    let mut txn = WriteTransaction::new(&env);

    let (batch_info, executed_txns) = accounts
        .commit(&mut txn, &transactions, &[reward.clone()], 2, 2)
        .unwrap();

    assert_eq!(
        batch_info,
        BatchInfo::new(receipts.clone(), tx_logs.clone(), inherent_logs.clone(),)
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10)
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000 + 10000 - 10)
    );

    assert_ne!(hash1, accounts.get_root_hash_assert(None));

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.revert(
            &mut txn,
            &executed_txns,
            &[reward],
            2,
            2,
            &Receipts::from(receipts.clone())
        ),
        Ok(BatchInfo::new(vec![], tx_logs, inherent_logs))
    );

    txn.commit();

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(hash1, accounts.get_root_hash_assert(None));
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
        accounts
            .get(&KeyNibbles::from(&address_validator_1), None)
            .unwrap(),
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
        accounts
            .get(&KeyNibbles::from(&address_validator_2), None)
            .unwrap(),
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
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000) - value1 - fee1 - value2 - fee2
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_validator_2), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000) + fee1 + fee2
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient_1), None)
            .unwrap()
            .unwrap()
            .balance(),
        value1
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient_2), None)
            .unwrap()
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

    let hash1 = accounts.get_root_hash_assert(None);

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap(),
        None
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    // Fails as sender address does not exist!
    // Note this kind of transaction would be rejected by the mempool
    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 1, 1)
            .is_err());
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap(),
        None
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    assert_eq!(hash1, accounts.get_root_hash_assert(None));

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
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    let hash2 = accounts.get_root_hash_assert(None);

    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = Coin::from_u64_unchecked(1000000);

    {
        let mut txn = WriteTransaction::new(&env);

        let (_, executed_txns) = accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 2, 2)
            .unwrap();

        assert_eq!(executed_txns, vec![ExecutedTransaction::Err(tx.clone())]);
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    assert_eq!(hash2, accounts.get_root_hash_assert(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from_u64_unchecked(5010);

    let mut tx2 = tx.clone();

    tx2.value += Coin::from_u64_unchecked(10);

    {
        let mut txn = WriteTransaction::new(&env);

        let (_, executed_txns) = accounts
            .commit(&mut txn, &vec![tx.clone(), tx2.clone()], &[reward], 2, 2)
            .unwrap();

        assert_eq!(
            executed_txns,
            vec![
                ExecutedTransaction::Ok(tx.clone()),
                ExecutedTransaction::Err(tx2.clone())
            ]
        );
    }

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_sender), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&address_recipient), None)
            .unwrap(),
        None
    );

    assert_eq!(hash2, accounts.get_root_hash_assert(None));
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
        target: address_validator,
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

    let total_txns = num_batches * Policy::blocks_per_batch() * num_txns;

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);

    let sender_balances = vec![100_000_000_000; 1];
    let recipient_balances = vec![0; total_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();
    let rewards = vec![];

    // Generate accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    let total_blocks = num_batches * Policy::blocks_per_batch();

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

        for _blocks in 0..Policy::blocks_per_batch() {
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

    let total_txns = num_batches * Policy::blocks_per_batch() * num_txns;

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);

    let sender_balances = vec![100_000_000_000; total_txns as usize];
    let recipient_balances = vec![10; total_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();
    let rewards = vec![];

    // Generate accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, true);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    let total_blocks = num_batches * Policy::blocks_per_batch();

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

        for _blocks in 0..Policy::blocks_per_batch() {
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

#[test]
fn it_commits_valid_and_failing_txns() {
    let priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();

    let key_pair = KeyPair::from(priv_key);

    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(env.clone());

    let mut db_txn = WriteTransaction::new(&env);

    // We need to populate the acccounts trie with different testing accounts
    let key_1 = KeyNibbles::from(&Address::from(&key_pair.public));

    // Basic account where fees are deducted for failing txns
    accounts
        .tree
        .put(
            &mut db_txn,
            &key_1,
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(1000),
            }),
        )
        .expect("complete trie");

    // Vesting Contract
    let start_contract = VestingContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_pair.public),
        start_time: 0,
        time_step: 100,
        step_amount: 100.try_into().unwrap(),
        total_amount: 1000.try_into().unwrap(),
    };

    accounts
        .tree
        .put(
            &mut db_txn,
            &KeyNibbles::from(&[1u8; 20][..]),
            Account::Vesting(start_contract),
        )
        .expect("complete trie");

    db_txn.commit();

    let mut db_txn = WriteTransaction::new(&env);
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
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let (_, executed_txns) = accounts
        .commit(&mut db_txn, &vec![tx.clone()], &[], 1, 200)
        .unwrap();

    assert_eq!(executed_txns, vec![ExecutedTransaction::Err(tx.clone())]);

    db_txn.commit();

    // We should deduct the fee from the vesting contract balance
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&[1u8; 20][..]), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(800)
    );

    //The fee should not be deducted from the sender
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&Address::from(&key_pair.public)), None)
            .unwrap()
            .unwrap()
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
    tx.sender_type = AccountType::Basic;

    let mut db_txn = WriteTransaction::new(&env);

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let (_, executed_txns) = accounts
        .commit(&mut db_txn, &vec![tx.clone()], &[], 1, 200)
        .unwrap();

    assert_eq!(executed_txns, vec![ExecutedTransaction::Err(tx.clone())]);

    db_txn.commit();

    //The fee should be deducted from the sender
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&Address::from(&key_pair.public)), None)
            .unwrap()
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(800)
    );
}
