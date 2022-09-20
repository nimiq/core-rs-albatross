use std::sync::Arc;

use parking_lot::RwLock;
use rand::rngs::StdRng;
use rand::SeedableRng;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use beserial::{Deserialize, Serialize};
use nimiq_block::{Block, MicroBlock, MicroBody, MicroHeader};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, PushResult};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{
    Address, KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey,
    PublicKey as SchnorrPublicKey, SecureGenerate,
};
use nimiq_mempool::mempool::Mempool;
use nimiq_mempool::{config::MempoolConfig, mempool_transactions::TxPriority};
use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
use nimiq_primitives::{networks::NetworkId, policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks_with_txns, signing_key, voting_key},
    test_transaction::{generate_accounts, generate_transactions, TestTransaction},
};
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::VrfSeed;

const NUM_TXNS_START_STOP: usize = 100;

pub const ACCOUNT_SECRET_KEY: &str =
    "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";

pub const VALIDATOR_SECRET_KEY: &str =
    "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";

const STAKER_ADDRESS: &str = "NQ20TSB0DFSMUH9C15GQGAGJTTE4D3MA859E";
const VALIDATOR_ADDRESS: &str = "NQ20 TSB0 DFSM UH9C 15GQ GAGJ TTE4 D3MA 859E";

fn ed25519_key_pair(secret_key: &str) -> SchnorrKeyPair {
    let priv_key: SchnorrPrivateKey =
        Deserialize::deserialize(&mut &hex::decode(secret_key).unwrap()[..]).unwrap();
    priv_key.into()
}

async fn send_get_mempool_txns(
    blockchain: Arc<RwLock<Blockchain>>,
    transactions: Vec<Transaction>,
    txn_len: usize,
) -> (Vec<Transaction>, usize) {
    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    send_txn_to_mempool(&mempool, mock_network, mock_id, transactions).await;

    // Get the transactions from the mempool
    mempool.get_transactions_for_block(txn_len)
}

async fn send_txn_to_mempool(
    mempool: &Mempool,
    mock_network: Arc<MockNetwork>,
    mock_id: MockId<MockPeerId>,
    transactions: Vec<Transaction>,
) {
    // Create a MPSC channel to directly send transactions to the mempool
    let (txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_executor_with_txn_stream::<MockNetwork>(
            Box::pin(ReceiverStream::new(txn_stream_rx)),
            mock_network,
        )
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
    mempool.stop_executor_without_unsubscribe().await;
}

async fn send_control_txn_to_mempool(
    mempool: &Mempool,
    mock_network: Arc<MockNetwork>,
    mock_id: MockId<MockPeerId>,
    transactions: Vec<Transaction>,
) {
    // Create a MPSC channel to directly send transactions to the mempool
    let (txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_control_executor_with_txn_stream::<MockNetwork>(
            Box::pin(ReceiverStream::new(txn_stream_rx)),
            mock_network,
        )
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
    mempool.stop_control_executor_without_unsubscribe().await;
}

async fn multiple_start_stop_send(
    blockchain: Arc<RwLock<Blockchain>>,
    transactions: Vec<Transaction>,
) {
    // Create a MPSC channel to directly send transactions to the mempool
    let (txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_executor_with_txn_stream::<MockNetwork>(
            Box::pin(ReceiverStream::new(txn_stream_rx)),
            mock_network,
        )
        .await;

    // Send the transactions
    let txn_stream_tx1 = txn_stream_tx.clone();
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
    mempool.stop_executor_without_unsubscribe().await;

    // Get the transactions from the mempool
    let (obtained_txns, _) = mempool.get_transactions_for_block(usize::MAX);

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
    mempool.stop_executor_without_unsubscribe().await;

    // We should not obtain any, since the executor should not be running.
    let (obtained_txns, _) = mempool.get_transactions_for_block(usize::MAX);

    // We should obtain 0 transactions
    assert_eq!(obtained_txns.len(), 0_usize);

    // Restart the executor
    // Create a MPSC channel to directly send transactions to the mempool
    let (txn_stream_tx, txn_stream_rx) = mpsc::channel(64);

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Subscribe mempool with the mpsc stream created
    mempool
        .start_executor_with_txn_stream::<MockNetwork>(
            Box::pin(ReceiverStream::new(txn_stream_rx)),
            mock_network,
        )
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
    mempool.stop_executor_without_unsubscribe().await;

    // Get the transactions from the mempool
    let (obtained_txns, _) = mempool.get_transactions_for_block(usize::MAX);

    // We should obtain same number of txns
    assert_eq!(obtained_txns.len(), NUM_TXNS_START_STOP);
}

fn create_dummy_micro_block(transactions: Option<Vec<Transaction>>) -> Block {
    // Build a dummy MicroHeader
    let micro_header = MicroHeader {
        version: 0,
        block_number: 0,
        view_number: 0,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![0; 1],
        state_root: Blake2bHash::default(),
        body_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let micro_body = transactions.map(|txns| MicroBody {
        fork_proofs: vec![],
        transactions: txns,
    });

    let micro_block = MicroBlock {
        header: micro_header,
        body: micro_body,
        justification: None,
    };
    Block::Micro(micro_block)
}

#[test(tokio::test)]
async fn push_same_tx_twice() {
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
        let mempool_transaction = TestTransaction {
            fee: 0,
            value: 10,
            recipient: recipient_accounts[0].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

    // Expect only 1 of the transactions in the mempool
    assert_eq!(txns.len(), 1);
}

#[test(tokio::test)]
async fn valid_tx_not_in_blockchain() {
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
        let mempool_transaction = TestTransaction {
            fee: 0,
            value: 10,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Create an empty blockchain
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Send 2 transactions
    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

    // Expect no transactions in the mempool
    assert_eq!(txns.len(), 0);
}

#[test(tokio::test)]
async fn push_tx_with_wrong_signature() {
    // Generate and sign transaction from an address
    let mut rng = StdRng::seed_from_u64(0);
    let sender_balances = vec![10000; 1];
    let recipient_balances = vec![0; 1];
    let mut genesis_builder = GenesisBuilder::default();

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    let mempool_transaction = TestTransaction {
        fee: 0,
        value: 10,
        recipient: recipient_accounts[0].clone(),
        sender: sender_accounts[0].clone(),
    };

    let (mut txns, txns_len) = generate_transactions(vec![mempool_transaction], true);
    log::debug!("Done generating transactions and accounts");
    txns[0].proof = hex::decode("0222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0003fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30b").unwrap();

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

    // Expect no transactions in the mempool
    assert_eq!(txns.len(), 0);
}

#[test(tokio::test)]
async fn mempool_get_txn_max_size() {
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance / num_txns,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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
    let (rec_txns, txn_size) =
        send_get_mempool_txns(blockchain.clone(), txns.clone(), txns_len - 1).await;

    // Expect only 1 of the transactions because of the size we passed
    assert_eq!(rec_txns.len(), 1);
    assert_eq!(txn_size, rec_txns[0].serialized_size());

    // Send the transactions again
    let (rec_txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

    // Expect both transactions
    assert_eq!(rec_txns.len(), num_txns as usize);
}

#[test(tokio::test)]
async fn mempool_get_txn_ordered() {
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance / num_txns,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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
    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

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

#[test(tokio::test)]
async fn push_tx_with_insufficient_balance() {
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: txns_value[i as usize],
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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
    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

    // Expect only 1 of the transactions in the mempool
    // The other one shouldn't be allowed because of insufficient balance
    assert_eq!(txns.len(), 1);
}

#[test(tokio::test)]
async fn multiple_transactions_multiple_senders() {
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance / num_txns,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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
    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

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

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_tps() {
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, txns_len) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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
    let (txns, _) = send_get_mempool_txns(blockchain, txns, txns_len).await;

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

#[test(tokio::test)]
async fn multiple_start_stop() {
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_update() {
    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate and sign transactions
    let balance = 100;
    let num_txns = 30;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + num_txns * num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

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
    let (txns, _) = generate_transactions(mempool_transactions, true);
    let transactions = txns.clone();
    log::debug!("Done generating transactions and accounts");

    // Build a couple of blocks with reverted transactions
    let balance = 100;
    let num_txns = 5;
    let mut reverted_transactions = vec![];
    let sender_balances = vec![balance + 100 + num_txns * num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: (i + 100) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        reverted_transactions.push(mempool_transaction);
    }
    let (mut rev_txns, _) = generate_transactions(reverted_transactions, true);
    rev_txns.extend_from_slice(&transactions[3..8]);
    let mut reverted_micro_blocks = vec![];
    reverted_micro_blocks.push((Blake2bHash::default(), create_dummy_micro_block(None)));
    reverted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(rev_txns[..5].to_vec())),
    ));
    reverted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(rev_txns[5..].to_vec())),
    ));
    log::debug!("Done generating reverted micro block");

    // Build a couple of blocks with adopted transactions
    let balance = 100;
    let num_txns = 5;
    let mut adopted_transactions = vec![];
    let sender_balances = vec![balance + 200 + num_txns * num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: (i + 200) as u64,
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        adopted_transactions.push(mempool_transaction);
    }
    let (mut adopted_txns, _) = generate_transactions(adopted_transactions, true);
    adopted_txns.extend_from_slice(&transactions[13..18]);
    let mut adopted_micro_blocks = vec![];
    adopted_micro_blocks.push((Blake2bHash::default(), create_dummy_micro_block(None)));
    adopted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(adopted_txns[..5].to_vec())),
    ));
    adopted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(adopted_txns[5..].to_vec())),
    ));

    log::debug!("Done generating adopted micro block");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    // Call mempool update
    mempool.mempool_update(&adopted_micro_blocks[..], &reverted_micro_blocks[..]);

    // Get txns from mempool
    let (updated_txns, _) = mempool.get_transactions_for_block(10_000);

    // Expect at least the original 30 transactions plus or minus:
    // - minus 5 from the adopted blocks since 5/10 transactions were in the mempool and they need to be dropped.
    // - plus 5 from the reverted blocks since 5/10 transactions were not in the mempool and we need them there.
    // Build a vector with exactly the transactions we were expecting (ordered by fee)
    let mut expected_txns = vec![];
    expected_txns.extend_from_slice(&transactions[..13]);
    expected_txns.extend_from_slice(&transactions[18..]);
    expected_txns.extend_from_slice(&rev_txns[..5]);
    expected_txns.reverse();

    assert_eq!(
        updated_txns.len(),
        expected_txns.len(),
        "Number of txns is not what is expected"
    );

    // Check transactions are sorted
    let mut prev_txn = updated_txns.first().expect("Is vector empty?").clone();
    for i in 0..updated_txns.len() {
        assert!(
            prev_txn.fee >= updated_txns[i].fee,
            "Transactions in mempool are not ordered by fee"
        );
        prev_txn = updated_txns[i].clone();
        assert_eq!(
            expected_txns[i], updated_txns[i],
            "Transaction at position {} is not expected",
            i
        );
    }
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
#[ignore]
// The purpose of this test is to verify that aged transactions, that is,
// transactions that are stored in the mempool for which the validity
// window is already expired, are properly pruned from the mempool.
// The test is marked as ignored because it takes some time to build a chain
// that produces more than TRANSACTION_VALIDITY_WINDOW blocks, however, one
// can easily change this parameter to some low number for testing purposes.
async fn mempool_update_aged_transaction() {
    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate and sign transactions
    let balance = 100;
    let num_txns = 30;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: 0 as u64,
            value: 60,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    // Build empty blocks just to advance the chain

    let mut adopted_micro_blocks = vec![];

    adopted_micro_blocks.push((Blake2bHash::default(), create_dummy_micro_block(None)));

    log::debug!("Done generating adopted micro block");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    // Create mempool and subscribe with a custom txn stream
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    assert_eq!(
        mempool.num_transactions(),
        30,
        "Number of txns in the mempools is not what is expected"
    );

    // We need a block producer to produce blocks
    let producer = BlockProducer::new(signing_key(), voting_key());

    let macro_blocks_to_be_produced =
        policy::TRANSACTION_VALIDITY_WINDOW / policy::BLOCKS_PER_BATCH;

    // Now we produce blocks past the transaction validity window
    produce_macro_blocks_with_txns(
        &producer,
        &blockchain,
        (macro_blocks_to_be_produced + 1).try_into().unwrap(),
        0,
        0,
    );

    // Call mempool update, this should prune all the old transactions
    mempool.mempool_update(&[].to_vec(), &[].to_vec());

    assert_eq!(
        mempool.num_transactions(),
        0,
        "Number of txns in the mempools is not what is expected"
    );

    // Get txns from mempool
    let (updated_txns, _) = mempool.get_transactions_for_block(10_000);

    // Should obtain 0 txns, as they are no longer valid due to aging
    assert_eq!(
        updated_txns.len(),
        0,
        "Number of txns is not what is expected"
    );
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_update_not_enough_balance() {
    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate and sign transactions
    let balance = 100;
    let num_txns = 30;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: 0 as u64,
            value: 60,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    // Build a couple of blocks with adopted transactions
    let num_txns = 10;
    let mut adopted_transactions = vec![];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Use same senders as before, because we want to test that the sender doesn't have enough funds to pay all pending txns

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: 0 as u64,
            value: 60,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        adopted_transactions.push(mempool_transaction);
    }
    let (adopted_txns, _) = generate_transactions(adopted_transactions, true);

    let mut adopted_micro_blocks = vec![];

    adopted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(adopted_txns.to_vec())),
    ));

    log::debug!("Done generating adopted micro block");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    // Create mempool and subscribe with a custom txn stream
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    assert_eq!(
        mempool.num_transactions(),
        30,
        "Number of txns in the mempools is not what is expected"
    );

    // We need a block producer to produce blocks
    let producer = BlockProducer::new(signing_key(), voting_key());

    {
        let bc = blockchain.upgradable_read();

        let block = producer.next_micro_block(
            &bc,
            bc.time.now(),
            0,
            None,
            vec![],
            adopted_txns,
            vec![0x41],
        );

        assert_eq!(
            Blockchain::push(bc, Block::Micro(block)),
            Ok(PushResult::Extended)
        );
    }

    // Call mempool update
    mempool.mempool_update(&adopted_micro_blocks[..], &[].to_vec());

    // Get txns from mempool
    let (updated_txns, _) = mempool.get_transactions_for_block(10_000);

    // Expect only 20 transations because in the adopted blocks we included 10 txns that would cause the senders
    // to not have enough balance to pay for all txns already in the mempool
    assert_eq!(
        updated_txns.len(),
        20,
        "Number of txns is not what is expected"
    );

    {
        let bc = blockchain.upgradable_read();

        let block = producer.next_micro_block(
            &bc,
            bc.time.now(),
            0,
            None,
            vec![],
            updated_txns.clone(),
            vec![0x41],
        );

        // We should succeed producing a block with the remaining mempool transactions
        assert_eq!(
            Blockchain::push(bc, Block::Micro(block)),
            Ok(PushResult::Extended)
        );
    }
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_update_pruned_account() {
    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate and sign transactions
    let balance = 100;
    let num_txns = 30;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Generate transactions
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: 0 as u64,
            value: 60,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    // Build a couple of blocks with adopted transactions
    let num_txns = 10;
    let mut adopted_transactions = vec![];
    let recipient_balances = vec![0; num_txns as usize];

    // Generate recipient accounts
    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    // Use same senders as before, because we want to test that the sender doesn't have enough funds to pay all pending txns

    // Generate transactions, these transactions will consume all the sender balance, which will cause the sender to be pruned
    // from the accounts tree
    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: 0 as u64,
            value: 100,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        adopted_transactions.push(mempool_transaction);
    }
    let (adopted_txns, _) = generate_transactions(adopted_transactions, true);

    let mut adopted_micro_blocks = vec![];

    adopted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(adopted_txns.to_vec())),
    ));

    log::debug!("Done generating adopted micro block");

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    assert_eq!(
        mempool.num_transactions(),
        30,
        "Number of txns in the mempools is not what is expected"
    );

    // We need a block producer to produce blocks
    let producer = BlockProducer::new(signing_key(), voting_key());

    {
        let bc = blockchain.upgradable_read();

        let block = producer.next_micro_block(
            &bc,
            bc.time.now(),
            0,
            None,
            vec![],
            adopted_txns,
            vec![0x41],
        );

        assert_eq!(
            Blockchain::push(bc, Block::Micro(block)),
            Ok(PushResult::Extended)
        );
    }

    // Call mempool update
    mempool.mempool_update(&adopted_micro_blocks[..], &[].to_vec());

    // Get txns from mempool
    let (updated_txns, _) = mempool.get_transactions_for_block(10_000);

    assert_eq!(
        updated_txns.len(),
        20,
        "Number of txns is not what is expected"
    );

    {
        let bc = blockchain.upgradable_read();

        let block = producer.next_micro_block(
            &bc,
            bc.time.now(),
            0,
            None,
            vec![],
            updated_txns.clone(),
            vec![0x41],
        );

        // We should succeed producing a block with the remaining mempool transactions
        assert_eq!(
            Blockchain::push(bc, Block::Micro(block)),
            Ok(PushResult::Extended)
        );
    }
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_update_create_staker_twice() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    // This is the transaction produced in the block
    let tx = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address.clone()),
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    // This is a slightly different transaction, that also creates the same staker
    let tx_dup = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address),
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let txns = vec![tx_dup];

    let adopted_txns = vec![tx];

    let mut adopted_micro_blocks = vec![];

    adopted_micro_blocks.push((
        Blake2bHash::default(),
        create_dummy_micro_block(Some(adopted_txns.to_vec())),
    ));

    log::debug!("Done generating adopted micro block");

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_control_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    assert_eq!(
        mempool.num_transactions(),
        1,
        "Number of txns in mempool is not what is expected"
    );

    // We need a block producer to produce blocks
    let producer = BlockProducer::new(signing_key(), voting_key());

    {
        let bc = blockchain.upgradable_read();

        let block = producer.next_micro_block(
            &bc,
            bc.time.now(),
            0,
            None,
            vec![],
            adopted_txns,
            vec![0x41],
        );

        assert_eq!(
            Blockchain::push(bc, Block::Micro(block)),
            Ok(PushResult::Extended)
        );
    }

    // Call mempool update
    mempool.mempool_update(&adopted_micro_blocks[..], &[].to_vec());

    // Get txns from mempool
    let (updated_txns, _) = mempool.get_control_transactions_for_block(10_000);

    assert_eq!(
        updated_txns.len(),
        0,
        "Number of txns is not what is expected"
    );

    let bc = blockchain.upgradable_read();

    let block = producer.next_micro_block(
        &bc,
        bc.time.now(),
        0,
        None,
        vec![],
        updated_txns.clone(),
        vec![0x41],
    );

    // We should succeed producing a block with the remaining mempool transactions
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_basic_prioritization_control_tx() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let validator_signing_key = ed25519_key_pair(VALIDATOR_SECRET_KEY);
    let address = Address::from_any_str(STAKER_ADDRESS).unwrap();
    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let unpark = TransactionBuilder::new_unpark_validator(
        &key_pair,
        validator_address,
        &validator_signing_key,
        1.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    // This is the transaction produced in the block
    let tx = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address.clone()),
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let txns = vec![tx];

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ));

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_control_txn_to_mempool(&mempool, mock_network, mock_id, txns.clone()).await;

    assert_eq!(
        mempool.num_transactions(),
        1,
        "Number of txns in mempool is not what is expected"
    );

    // Insert unpark with high priority
    mempool
        .add_transaction(unpark.clone(), Some(TxPriority::HighPriority))
        .await
        .unwrap();

    // Get control txns from mempool
    let (updated_txns, _) = mempool.get_control_transactions_for_block(10_000);

    // Now we should obtain one control transaction
    assert_eq!(
        updated_txns.len(),
        2,
        "Number of txns is not what is expected"
    );

    // We should obtain the txns in the reversed ordered as the unpark should have been prioritized.
    assert_eq!(updated_txns[0], unpark);

    // Now the mempool should have 0 total txns
    assert_eq!(
        mempool.num_transactions(),
        0,
        "Number of txns in mempool is not what is expected"
    );
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_regular_and_control_tx() {
    let mut rng = StdRng::seed_from_u64(0);
    let balance = 100_000_000;
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
        let mempool_transaction = TestTransaction {
            fee: (i + 1) as u64,
            value: 1,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[0].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }
    let (txns, _) = generate_transactions(mempool_transactions, true);
    log::debug!("Done generating transactions and accounts");

    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();

    // Add a validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    // Create mempool and subscribe with a custom txn stream.
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // This is the transaction produced in the block
    let control_tx = TransactionBuilder::new_create_staker(
        &sender_accounts[0].keypair,
        &sender_accounts[0].keypair,
        None,
        100.try_into().unwrap(),
        1.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let control_txns = vec![control_tx];

    // Send txns to mempool
    send_control_txn_to_mempool(
        &mempool,
        mock_network.clone(),
        mock_id.clone(),
        control_txns,
    )
    .await;

    assert_eq!(
        mempool.num_transactions(),
        1,
        "Number of txns in mempool is not what is expected"
    );

    // Get regular txns from mempool
    let (updated_txns, _) = mempool.get_transactions_for_block(10_000);

    //We should obtain 0 regular txns since we only have control txns in the mempool
    assert_eq!(
        updated_txns.len(),
        0,
        "Number of txns is not what is expected"
    );

    //Send regular txns to mempool
    send_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    // Get control txns from mempool
    let (updated_txns, _) = mempool.get_control_transactions_for_block(10_000);

    //Now we should obtain one control transaction
    assert_eq!(
        updated_txns.len(),
        1,
        "Number of txns is not what is expected"
    );

    // Get regular txns from mempool
    let (updated_txns, _) = mempool.get_transactions_for_block(10_000);

    //Now we should obtain all regular txns
    assert_eq!(
        updated_txns.len(),
        num_txns as usize,
        "Number of txns is not what is expected"
    );

    //Now the mempool should have 0 total txns
    assert_eq!(
        mempool.num_transactions(),
        0,
        "Number of txns in mempool is not what is expected"
    );
}

#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn mempool_update_create_staker_non_existant_delegation_addr() {
    let mut rng = StdRng::seed_from_u64(0);
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate and sign transactions
    let balance = 10000000;
    let num_txns = 10;

    let sender_balances = vec![balance; num_txns as usize];

    // Generate sender accounts
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    // Note that we are using generated accounts for the create staker transaction
    let tx = TransactionBuilder::new_create_staker(
        &sender_accounts[0].keypair,
        &sender_accounts[0].keypair,
        Some(Address::from(&sender_accounts[1].keypair)),
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let txns = vec![tx];

    // Add validator to genesis, note that the delegation adddress is not in the genesis
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
    );

    // Generate the genesis and blockchain
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

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

    // Create mempool and subscribe with a custom txn stream
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());
    let mut hub = MockHub::new();
    let mock_id = MockId::new(hub.new_address().into());
    let mock_network = Arc::new(hub.new_network());

    // Send txns to mempool
    send_control_txn_to_mempool(&mempool, mock_network, mock_id, txns).await;

    // The transaction should be rejected by the mempool, since the delegation address does not exist
    assert_eq!(
        mempool.num_transactions(),
        0,
        "Number of txns in the mempools is not what is expected"
    );
}

#[tokio::test]
async fn applies_total_tx_size_limits() {
    let env = VolatileEnvironment::new(10).unwrap();
    let mut genesis_builder = GenesisBuilder::default();

    // Generate transactions
    let balance = 1;
    let num_txns = 5;
    let mut mempool_transactions = vec![];
    let sender_balances = vec![balance + num_txns * num_txns; num_txns as usize];
    let recipient_balances = vec![0; num_txns as usize];

    let recipient_accounts = generate_accounts(recipient_balances, &mut genesis_builder, false);
    let sender_accounts = generate_accounts(sender_balances, &mut genesis_builder, true);

    for i in 0..num_txns {
        let mempool_transaction = TestTransaction {
            fee: if i < 2 { 0 as u64 } else { (i + 1) as u64 }, // Produce two tx with the same lowest fees
            value: balance,
            recipient: recipient_accounts[i as usize].clone(),
            sender: sender_accounts[i as usize].clone(),
        };
        mempool_transactions.push(mempool_transaction);
    }

    let (txns, txns_len) = generate_transactions(mempool_transactions, true);

    let mut rng = StdRng::seed_from_u64(0);
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            Arc::new(OffsetTime::new()),
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    // Create mempool with total size limit just below the total one of the generated transactions
    let mempool_config = MempoolConfig {
        size_limit: txns_len - 1,
        ..Default::default()
    };
    let mempool = Mempool::new(blockchain, mempool_config);

    // The worst transaction is the second transaction with the lowest fee.
    let worst_tx = txns[1].hash::<Blake2bHash>();

    for tx in txns {
        mempool.add_transaction(tx, None).await.unwrap();
    }

    let (mempool_txns, _) = mempool.get_transactions_for_block(txns_len);

    // We expect that the tx with the lowest fee did not stay in the mempool
    for tx in &mempool_txns {
        assert_ne!(tx.hash::<Blake2bHash>(), worst_tx);
    }
    assert_eq!(mempool_txns.len(), (num_txns - 1) as usize);
}
