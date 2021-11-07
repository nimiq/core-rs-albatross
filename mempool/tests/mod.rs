use std::sync::Arc;

use futures::{channel::mpsc, sink::SinkExt};
use log::LevelFilter::Debug;
use parking_lot::RwLock;
use rand::prelude::StdRng;
use rand::SeedableRng;

use beserial::{Deserialize, Serialize};
use nimiq_blockchain::Blockchain;
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_build_tools::genesis::GenesisBuilder;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_mempool::config::MempoolConfig;
use nimiq_mempool::mempool::Mempool;
use nimiq_network_mock::{MockHub, MockId, MockNetwork};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_utils::time::OffsetTime;

const BASIC_TRANSACTION: &str = "000222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0754d1260f15bea0e8fb07ab18f45301483599e34000000000000c350000000000000008a00019640023fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30a";
const ENABLE_LOG: bool = false;
const NUM_TXNS_START_STOP: usize = 400;

// Tests we want
// 1. ~Get transactions sorted by fee~
// 2. ~Get transactions max size test~
// 2. ~Multiple transactions from various senders~
// 3. Blockchain event related tests:
// 3.1 Mined block invalids a tx in the mempool
// 3.2 Mined Unknown TX in blockchain from a known sender
// 3.3 Mined block with unknown senders does nothing to the mempool
// 4. Rebranch operation, multiple blocks are reverted and new blocks are added
//

#[derive(Clone)]
struct MempoolAccount {
    keypair: KeyPair,
    address: Address,
}

#[derive(Clone)]
struct MempoolTransaction {
    fee: u64,
    value: u64,
    sender: MempoolAccount,
    recipient: MempoolAccount,
}

async fn send_txn_to_mempool(
    blockchain: Arc<RwLock<Blockchain>>,
    transactions: Vec<Transaction>,
    txn_len: usize,
) -> Vec<Transaction> {
    // Create a MPSC channel to directly send transactions to the mempool
    let (mut txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_executor_with_txn_stream::<MockNetwork>(Box::pin(txn_stream_rx), mock_network)
        .await;

    // Send the transactions
    tokio::task::spawn(async move {
        for txn in transactions {
            txn_stream_tx
                .send((txn.clone(), mock_id.clone()))
                .await
                .unwrap();
        }
    })
    .await
    .expect("Send failed");

    let timeout = tokio::time::Duration::from_secs(1);
    tokio::time::sleep(timeout).await;
    mempool.stop_executor();

    // Get the transactions from the mempool
    mempool
        .get_transactions_block(txn_len)
        .expect("expected transaction vec")
}

async fn multiple_start_stop_send(
    blockchain: Arc<RwLock<Blockchain>>,
    transactions: Vec<Transaction>,
) {
    // Create a MPSC channel to directly send transactions to the mempool
    let (mut txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_executor_with_txn_stream::<MockNetwork>(Box::pin(txn_stream_rx), mock_network)
        .await;

    // Send the transactions
    let mut txn_stream_tx1 = txn_stream_tx.clone();
    let mock_id1 = mock_id.clone();
    let txns = transactions.clone();
    tokio::task::spawn(async move {
        for txn in txns {
            txn_stream_tx1
                .send((txn.clone(), mock_id1.clone()))
                .await
                .unwrap();
        }
    })
    .await
    .expect("Send failed");

    let timeout = tokio::time::Duration::from_secs(2);
    tokio::time::sleep(timeout).await;
    mempool.stop_executor();

    // Get the transactions from the mempool
    let obtained_txns = mempool.get_transactions_block(usize::MAX).unwrap();

    // We should obtain the same amount of transactions
    assert_eq!(obtained_txns.len(), NUM_TXNS_START_STOP);

    // Now send more transactions via the transaction stream.
    let txns = transactions.clone();
    tokio::task::spawn(async move {
        for txn in txns {
            txn_stream_tx
                .send((txn.clone(), mock_id.clone()))
                .await
                .expect_err("Send should fail, executor is stopped");
        }
    })
    .await
    .expect("Send failed");

    let timeout = tokio::time::Duration::from_secs(2);
    tokio::time::sleep(timeout).await;

    // Call stop again, nothing should happen.
    mempool.stop_executor();

    // We should not obtain any, since the executor should not be running.
    let obtained_txns = mempool.get_transactions_block(usize::MAX).unwrap();

    // We should obtain 0 transactions
    assert_eq!(obtained_txns.len(), 0_usize);

    // Restart the executor
    // Create a MPSC channel to directly send transactions to the mempool
    let (mut txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_executor_with_txn_stream::<MockNetwork>(Box::pin(txn_stream_rx), mock_network)
        .await;

    // Send the transactions
    let txns = transactions.clone();
    tokio::task::spawn(async move {
        for txn in txns {
            txn_stream_tx
                .send((txn.clone(), mock_id.clone()))
                .await
                .unwrap();
        }
    })
    .await
    .expect("Send failed");

    let timeout = tokio::time::Duration::from_secs(2);
    tokio::time::sleep(timeout).await;
    mempool.stop_executor();

    // Get the transactions from the mempool
    let obtained_txns = mempool.get_transactions_block(usize::MAX).unwrap();

    // We should obtain same number of txns
    assert_eq!(obtained_txns.len(), NUM_TXNS_START_STOP);
}

fn generate_accounts(
    balances: Vec<u64>,
    genesis_builder: &mut GenesisBuilder,
    add_to_genesis: bool,
) -> Vec<MempoolAccount> {
    let mut mempool_accounts = vec![];

    for i in 0..balances.len() as usize {
        // Generate the txns_sender and txns_rec vectors to later generate transactions
        let keypair = KeyPair::generate_default_csprng();
        let address = Address::from(&keypair.public);
        let mempool_account = MempoolAccount {
            keypair,
            address: address.clone(),
        };
        mempool_accounts.push(mempool_account);

        if add_to_genesis {
            // Add accounts to the genesis builder
            genesis_builder.with_basic_account(address, Coin::from_u64_unchecked(balances[i]));
        }
    }
    mempool_accounts
}

fn generate_transactions(
    mempool_transactions: Vec<MempoolTransaction>,
) -> (Vec<Transaction>, usize) {
    let mut txns_len = 0;
    let mut txns: Vec<Transaction> = vec![];

    log::debug!("Generating transactions and accounts");

    for mempool_transaction in mempool_transactions {
        // Generate transactions
        let mut txn = Transaction::new_basic(
            mempool_transaction.sender.address.clone(),
            mempool_transaction.recipient.address.clone(),
            Coin::from_u64_unchecked(mempool_transaction.value),
            Coin::from_u64_unchecked(mempool_transaction.fee),
            1,
            NetworkId::UnitAlbatross,
        );

        let signature_proof = SignatureProof::from(
            mempool_transaction.sender.keypair.public,
            mempool_transaction
                .sender
                .keypair
                .sign(&txn.serialize_content()),
        );

        txn.proof = signature_proof.serialize_to_vec();
        txns.push(txn.clone());
        txns_len += txn.serialized_size();
    }
    (txns, txns_len)
}

#[tokio::test]
async fn push_same_tx_twice() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);
    let num_txns = 2;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![10000; 1];
    let recipient_balances = vec![0; 1];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for _ in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: 0,
            value: 10,
            recipient: recipient_accounts[0].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    let txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect only 1 of the transactions in the mempool
    assert_eq!(txns.len(), 1);
}

#[tokio::test]
async fn valid_tx_not_in_blockchain() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from an address
    let balance = 40;
    let num_txns = 2;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + 3; 1];
    let recipient_balances = vec![0; num_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, false);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: 0,
            value: 10,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Create an empty blockchain
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Send 2 transactions
    let txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect no transactions in the mempool
    assert_eq!(txns.len(), 0);
}

#[tokio::test]
async fn push_tx_with_wrong_signature() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Create an empty blockchain
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Build transaction with invalid signature from serialized data
    let serialized_txn: Vec<u8> = hex::decode(BASIC_TRANSACTION).unwrap();
    let mut txn: Transaction = Deserialize::deserialize(&mut &serialized_txn[..]).unwrap();
    // last char a (valid) -> b
    txn.proof = hex::decode("0222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0003fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30b").unwrap();
    let txn_len = txn.serialized_size();
    let txns = vec![txn; 2];
    let txns = send_txn_to_mempool(blockchain, txns, txn_len * 2).await;

    // Expect no transactions in the mempool
    assert_eq!(txns.len(), 0);
}

#[tokio::test]
async fn mempool_get_txn_max_size() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);
    let balance = 40;
    let num_txns = 2;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + 3; 1];
    let recipient_balances = vec![0; num_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: (i + 1) as u64,
            value: balance / num_txns,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Send the transactions
    let rec_txns = send_txn_to_mempool(blockchain.clone(), txns.clone(), txns_len - 1).await;

    // Expect only 1 of the transactions because of the size we passed
    // The other one shouldn't be allowed because of insufficient balance
    assert_eq!(rec_txns.len(), 1);

    // Send the transactions again
    let rec_txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect both transactions
    assert_eq!(rec_txns.len(), num_txns as usize);
}

#[tokio::test]
async fn mempool_get_txn_ordered() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);
    let balance = 40;
    let num_txns = 4;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + num_txns * 3; 1];
    let recipient_balances = vec![0; num_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: (i + 1) as u64,
            value: balance / num_txns,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Send the transactions
    let txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect all of the transactions in the mempool
    assert_eq!(txns.len(), num_txns as usize);
    // Check transactions are sorted
    let mut prev_txn = txns.first().expect("Is vector empty?").clone();
    for txn in txns {
        assert!(
            prev_txn.fee >= txn.fee,
            "Transactions in mempool are not ordered by fee"
        );
        prev_txn = txn.clone();
    }
}

#[tokio::test]
async fn push_tx_with_insufficient_balance() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);
    let balance = 25;
    let num_txns = 3;
    let txns_value: Vec<u64> = vec![balance, balance / (num_txns - 1), balance / (num_txns - 1)];
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance; 1];
    let recipient_balances = vec![0; num_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: (i + 1) as u64,
            value: txns_value[i as usize],
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Send the transactions
    let txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect only 1 of the transactions in the mempool
    // The other one shouldn't be allowed because of insufficient balance
    assert_eq!(txns.len(), 1);
}

#[tokio::test]
async fn multiple_transactions_multiple_senders() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    let mut rng = StdRng::seed_from_u64(0);
    let balance = 40;
    let num_txns = 9;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + num_txns * num_txns / num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: (i + 1) as u64,
            value: balance / num_txns,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Send the transactions
    let txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect all of the transactions in the mempool
    assert_eq!(txns.len(), num_txns as usize);
    // Check transactions are sorted
    let mut prev_txn = txns.first().expect("Is vector empty?").clone();
    for txn in txns {
        assert!(
            prev_txn.fee >= txn.fee,
            "Transactions in mempool are not ordered by fee"
        );
        prev_txn = txn.clone();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 10)]
async fn mempool_tps() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate and sign transaction from address_a using a balance that will be used to create the account later
    let balance = 100;
    let num_txns = 3_200;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + num_txns * num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: (i + 1) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Send the transactions
    let txns = send_txn_to_mempool(blockchain, txns, txns_len).await;

    // Expect at least 100 of the transactions in the mempool
    assert!(
        txns.len() > 100,
        "Min TPS of 100 wasn't achieved: TPS obtained {}",
        txns.len()
    );
    println!("Mempool processed {} TPS", txns.len());
    // Check transactions are sorted
    let mut prev_txn = txns.first().expect("Is vector empty?").clone();
    for txn in txns {
        assert!(
            prev_txn.fee >= txn.fee,
            "Transactions in mempool are not ordered by fee"
        );
        prev_txn = txn.clone();
    }
}

#[tokio::test]
async fn multiple_start_stop() {
    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    log::debug!("Generating transactions and accounts");

    let balance = 100;
    let num_txns = NUM_TXNS_START_STOP as u64;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + num_txns * num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = MempoolTransaction {
            fee: (i + 1) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions);
    log::debug!("Done generating transactions and accounts");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Address::from([0u8; 20]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate().unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Send the transactions
    multiple_start_stop_send(blockchain, txns).await;
}
