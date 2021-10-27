use std::sync::Arc;

use futures::{channel::mpsc, sink::SinkExt};
use log::LevelFilter::Debug;
//use nimiq_mempool::filter::MempoolFilter;
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
    });

    let timeout = tokio::time::Duration::from_secs(1);
    tokio::time::sleep(timeout).await;
    mempool.stop_executor();

    // Get the transactions from the mempool
    mempool
        .get_transactions_block(txn_len)
        .expect("expected transaction vec")
}

#[tokio::test]
async fn push_same_tx_twice() {
    let mut rng = StdRng::seed_from_u64(0);
    let keypair_a = KeyPair::generate_default_csprng();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from address_a
    let mut txn = Transaction::new_basic(
        address_a.clone(),
        address_b,
        Coin::from_u64_unchecked(10),
        Coin::from_u64_unchecked(0),
        1,
        NetworkId::UnitAlbatross,
    );

    let signature_proof =
        SignatureProof::from(keypair_a.public, keypair_a.sign(&txn.serialize_content()));

    txn.proof = signature_proof.serialize_to_vec();
    let txn_len = txn.serialized_size();

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Build a blockchain with a basic account and a validator
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_basic_account(address_a, Coin::from_u64_unchecked(10000));
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

    // Send twice the same transaction
    let txns = vec![txn; 2];
    let txns = send_txn_to_mempool(blockchain, txns, txn_len).await;

    // Expect only 1 of the transactions in the mempool
    assert_eq!(txns.len(), 1);
}

#[tokio::test]
async fn valid_tx_not_in_blockchain() {
    let keypair_a = KeyPair::generate_default_csprng();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from address_a
    let mut txn = Transaction::new_basic(
        address_a,
        address_b,
        Coin::from_u64_unchecked(10),
        Coin::from_u64_unchecked(0),
        1,
        NetworkId::UnitAlbatross,
    );

    let signature_proof =
        SignatureProof::from(keypair_a.public, keypair_a.sign(&txn.serialize_content()));

    txn.proof = signature_proof.serialize_to_vec();
    let txn_len = txn.serialized_size();

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Create an empty blockchain
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Send 2 transactions
    let txns = vec![txn; 2];
    let txns = send_txn_to_mempool(blockchain, txns, txn_len * 2).await;

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
    let mut rng = StdRng::seed_from_u64(0);
    let keypair_a = KeyPair::generate_default_csprng();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from address_a using a balance that will be used to create the account later
    let balance = 40;
    let mut txns_len = 0;
    let num_txns = 2;
    let txns_value: Vec<u64> = vec![balance / num_txns, balance / num_txns];
    let txns_fee: Vec<u64> = (1..num_txns + 1).collect();
    let mut txns: Vec<Transaction> = vec![];
    for i in 0..num_txns as usize {
        let mut txn = Transaction::new_basic(
            address_a.clone(),
            address_b.clone(),
            Coin::from_u64_unchecked(txns_value[i]),
            Coin::from_u64_unchecked(txns_fee[i]),
            1,
            NetworkId::UnitAlbatross,
        );

        let signature_proof =
            SignatureProof::from(keypair_a.public, keypair_a.sign(&txn.serialize_content()));

        txn.proof = signature_proof.serialize_to_vec();
        txns.push(txn.clone());
        txns_len += txn.serialized_size();
    }

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Build a blockchain with a basic account (using the balance of the tx) and a validator
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_basic_account(address_a, Coin::from_u64_unchecked(balance + 3));
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
    let mut rng = StdRng::seed_from_u64(0);
    let keypair_a = KeyPair::generate_default_csprng();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from address_a using a balance that will be used to create the account later
    let balance = 40;
    let mut txns_len = 0;
    let num_txns = 4;
    let txns_value: Vec<u64> = vec![balance / num_txns; num_txns as usize];
    let txns_fee: Vec<u64> = (1..num_txns + 1).collect();
    let mut txns: Vec<Transaction> = vec![];
    for i in 0..num_txns as usize {
        let mut txn = Transaction::new_basic(
            address_a.clone(),
            address_b.clone(),
            Coin::from_u64_unchecked(txns_value[i]),
            Coin::from_u64_unchecked(txns_fee[i]),
            1,
            NetworkId::UnitAlbatross,
        );

        let signature_proof =
            SignatureProof::from(keypair_a.public, keypair_a.sign(&txn.serialize_content()));

        txn.proof = signature_proof.serialize_to_vec();
        txns.push(txn.clone());
        txns_len += txn.serialized_size();
    }

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Build a blockchain with a basic account (using the balance of the tx) and a validator
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_basic_account(address_a, Coin::from_u64_unchecked(balance + num_txns * 3));
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
    let mut rng = StdRng::seed_from_u64(0);
    let keypair_a = KeyPair::generate_default_csprng();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from address_a using a balance that will be used to create the account later
    let balance = 25;
    let mut txns_len = 0;
    let num_txns = 3;
    let txns_value: Vec<u64> = vec![balance, balance / (num_txns - 1), balance / (num_txns - 1)];
    let txns_fee: Vec<u64> = (1..num_txns + 1).collect();
    let mut txns: Vec<Transaction> = vec![];
    for i in 0..num_txns as usize {
        let mut txn = Transaction::new_basic(
            address_a.clone(),
            address_b.clone(),
            Coin::from_u64_unchecked(txns_value[i]),
            Coin::from_u64_unchecked(txns_fee[i]),
            1,
            NetworkId::UnitAlbatross,
        );

        let signature_proof =
            SignatureProof::from(keypair_a.public, keypair_a.sign(&txn.serialize_content()));

        txn.proof = signature_proof.serialize_to_vec();
        txns.push(txn.clone());
        txns_len += txn.serialized_size();
    }

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Build a blockchain with a basic account (using the balance of the tx) and a validator
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_basic_account(address_a, Coin::from_u64_unchecked(balance));
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
    let mut rng = StdRng::seed_from_u64(0);
    let keypair_a = KeyPair::generate_default_csprng();
    let keypair_b = KeyPair::generate_default_csprng();
    let keypair_c = KeyPair::generate_default_csprng();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from(&keypair_b.public);
    let address_c = Address::from(&keypair_c.public);

    if ENABLE_LOG {
        simple_logger::SimpleLogger::new()
            .with_level(Debug)
            .init()
            .ok();
    }

    // Generate and sign transaction from address_a using a balance that will be used to create the account later
    let balance = 40;
    let mut txns_len = 0;
    let num_txns = 9;
    let txns_value: Vec<u64> = vec![balance / num_txns; num_txns as usize];
    let txns_fee: Vec<u64> = (1..num_txns + 1).collect();
    let mut txns: Vec<Transaction> = vec![];
    let mut txns_sender: Vec<Address> = vec![];
    let mut sender_keypair: Vec<KeyPair> = vec![];
    let mut txns_rec: Vec<Address> = vec![];

    // Generate the txns_sender and txns_rec vectors to later generate transactions
    for _ in 0..3 {
        txns_sender.push(address_a.clone());
        sender_keypair.push(keypair_a.clone());
        txns_rec.push(address_b.clone());
        txns_sender.push(address_b.clone());
        sender_keypair.push(keypair_b.clone());
        txns_rec.push(address_c.clone());
        txns_sender.push(address_c.clone());
        sender_keypair.push(keypair_c.clone());
        txns_rec.push(address_a.clone());
    }
    // Generate transactions
    for i in 0..num_txns as usize {
        let mut txn = Transaction::new_basic(
            txns_sender[i].clone(),
            txns_rec[i].clone(),
            Coin::from_u64_unchecked(txns_value[i]),
            Coin::from_u64_unchecked(txns_fee[i]),
            1,
            NetworkId::UnitAlbatross,
        );

        let signature_proof = SignatureProof::from(
            sender_keypair[i].public,
            sender_keypair[i].sign(&txn.serialize_content()),
        );

        txn.proof = signature_proof.serialize_to_vec();
        txns.push(txn.clone());
        txns_len += txn.serialized_size();
    }

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Build a blockchain with a basic account (using the balance of the tx) and a validator
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_basic_account(
        address_a,
        Coin::from_u64_unchecked(balance + num_txns * num_txns),
    );
    genesis_builder.with_basic_account(
        address_b,
        Coin::from_u64_unchecked(balance + num_txns * num_txns),
    );
    genesis_builder.with_basic_account(
        address_c,
        Coin::from_u64_unchecked(balance + num_txns * num_txns),
    );
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
    let mut txns_len = 0;
    let num_txns = 3_200;
    let txns_value: Vec<u64> = vec![balance; num_txns as usize];
    let txns_fee: Vec<u64> = (1..num_txns + 1).collect();
    let mut txns: Vec<Transaction> = vec![];
    let mut sender_addresses: Vec<Address> = vec![];
    let mut sender_keypairs: Vec<KeyPair> = vec![];
    let mut reciver_addresses: Vec<Address> = vec![];

    log::debug!("Generating transactions and accounts");

    for i in 0..num_txns as usize {
        // Generate the txns_sender and txns_rec vectors to later generate transactions
        let sender_keypair = KeyPair::generate_default_csprng();
        let receiver_keypair = KeyPair::generate_default_csprng();
        let sender_address = Address::from(&sender_keypair.public);
        let receiver_address = Address::from(&receiver_keypair.public);
        sender_keypairs.push(sender_keypair);
        sender_addresses.push(sender_address);
        reciver_addresses.push(receiver_address);

        // Generate transactions
        let mut txn = Transaction::new_basic(
            sender_addresses[i].clone(),
            reciver_addresses[i].clone(),
            Coin::from_u64_unchecked(txns_value[i]),
            Coin::from_u64_unchecked(txns_fee[i]),
            1,
            NetworkId::UnitAlbatross,
        );

        let signature_proof = SignatureProof::from(
            sender_keypairs[i].public,
            sender_keypairs[i].sign(&txn.serialize_content()),
        );

        txn.proof = signature_proof.serialize_to_vec();
        txns.push(txn.clone());
        txns_len += txn.serialized_size();

        // Add accounts to the genesis builder
        genesis_builder.with_basic_account(
            sender_addresses[i].clone(),
            Coin::from_u64_unchecked(balance + num_txns * num_txns),
        );
    }

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

    // Expect at least 300 of the transactions in the mempool
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
