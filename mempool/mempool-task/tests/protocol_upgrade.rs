use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, PushResult};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_mempool::config::MempoolConfig;
use nimiq_mempool_task::MempoolTask;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        fill_micro_blocks_with_txns, next_election_block_with_version,
        produce_macro_blocks_with_txns, signal_next_protocol_version_via_tx, signing_key,
        validator_address, voting_key,
    },
    node::Node,
    test_rng,
    test_transaction::generate_accounts,
};
use nimiq_time::timeout;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;

#[test(tokio::test)]
async fn mempool_task_syncs_protocol_version_while_unsynced() {
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    genesis_builder.with_genesis_block_number(Policy::genesis_block_number());

    genesis_builder.with_genesis_validator(
        validator_address(),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env).unwrap();
    let mut hub = Some(MockHub::default());
    let mut node = Node::<MockNetwork>::history_with_genesis_info(0, genesis_info, &mut hub).await;

    let blockchain = Arc::clone(&node.blockchain);
    let consensus = node.consensus.take().unwrap();
    let producer = BlockProducer::new(signing_key(), voting_key());

    // Advance to the last batch of the epoch and signal the protocol upgrade.
    produce_macro_blocks_with_txns(
        &producer,
        &blockchain,
        (Policy::batches_per_epoch() - 1) as usize,
        0,
        0,
    );
    let upgrade_version = signal_next_protocol_version_via_tx(&producer, &blockchain);
    fill_micro_blocks_with_txns(&producer, &blockchain, 0, 0);

    // The task caches the pre-upgrade version at construction. Consensus is deliberately
    // not established: the upgrade is observed while the node is still syncing.
    let mut mempool_task = MempoolTask::new(
        &consensus,
        Arc::clone(&blockchain),
        MempoolConfig::default(),
    );
    assert!(mempool_task.mempool.protocol_version() < upgrade_version);

    let upgrade_block = next_election_block_with_version(&producer, &blockchain, upgrade_version);
    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), upgrade_block),
        Ok(PushResult::Extended)
    );

    // Process events until the protocol upgrade is seen.
    loop {
        let event = timeout(Duration::from_secs(5), mempool_task.next())
            .await
            .unwrap()
            .unwrap();
        if matches!(
            BlockchainEvent::from(event),
            BlockchainEvent::ProtocolUpgrade(..)
        ) {
            break;
        }
    }
    assert_eq!(mempool_task.mempool.protocol_version(), upgrade_version);
}

#[test(tokio::test)]
async fn mempool_task_resyncs_protocol_version_on_activation() {
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    genesis_builder.with_genesis_block_number(Policy::genesis_block_number());

    genesis_builder.with_genesis_validator(
        validator_address(),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env).unwrap();
    let mut hub = Some(MockHub::default());
    let mut node = Node::<MockNetwork>::history_with_genesis_info(0, genesis_info, &mut hub).await;

    let blockchain = Arc::clone(&node.blockchain);
    let mut consensus = node.consensus.take().unwrap();

    let mut mempool_task = MempoolTask::new(
        &consensus,
        Arc::clone(&blockchain),
        MempoolConfig::default(),
    );

    // Simulate a stale cached version, e.g. from upgrade events that were dropped
    // by a lagging blockchain event stream.
    let current_version = blockchain.read().protocol_version();
    mempool_task
        .mempool
        .set_protocol_version(current_version - 1);

    // Establishing consensus starts the mempool, which must resync the version.
    consensus.force_established();
    let _ = timeout(Duration::from_millis(100), mempool_task.next()).await;

    assert_eq!(mempool_task.mempool.protocol_version(), current_version);
}

#[test(tokio::test)]
#[ignore = "Enable once a protocol-gated transaction verification rule exists"]
async fn mempool_task_transactions_evicted_after_protocol_upgrade() {
    // TODO: Once a protocol-gated transaction verification rule exists, add a
    // transaction that verifies before the protocol upgrade and fails
    // verification after it, then assert that it is evicted from the mempool
    // when the protocol upgrade is processed.
    todo!("Add a protocol-upgrade eviction test case once a real tx rule exists");
    let mut rng = test_rng(true);
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    genesis_builder.with_genesis_block_number(Policy::genesis_block_number());

    let sender_accounts = generate_accounts(vec![100, 100], &mut genesis_builder, true, &mut rng);
    let recipient_accounts = generate_accounts(vec![0, 0], &mut genesis_builder, false, &mut rng);

    genesis_builder.with_genesis_validator(
        validator_address(),
        signing_key().public,
        voting_key().public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env).unwrap();
    let mut hub = Some(MockHub::default());
    let mut node = Node::<MockNetwork>::history_with_genesis_info(0, genesis_info, &mut hub).await;

    let blockchain = Arc::clone(&node.blockchain);
    let mut consensus = node.consensus.take().unwrap();
    let producer = BlockProducer::new(signing_key(), voting_key());

    produce_macro_blocks_with_txns(
        &producer,
        &blockchain,
        (Policy::batches_per_epoch() - 1) as usize,
        0,
        0,
    );
    let upgrade_version = signal_next_protocol_version_via_tx(&producer, &blockchain);
    fill_micro_blocks_with_txns(&producer, &blockchain, 0, 0);

    let validity_start_height = blockchain.read().block_number() + 1;
    let txs: Vec<Transaction> = sender_accounts
        .iter()
        .zip(recipient_accounts.iter())
        .map(|(sender, recipient)| {
            TransactionBuilder::new_basic(
                &sender.keypair,
                recipient.address.clone(),
                Coin::from_u64_unchecked(10),
                Coin::ZERO,
                validity_start_height,
                NetworkId::UnitAlbatross,
            )
            .unwrap()
        })
        .collect();

    consensus.force_established();

    let mut mempool_task = MempoolTask::new(
        &consensus,
        Arc::clone(&blockchain),
        MempoolConfig::default(),
    );

    for tx in &txs {
        mempool_task
            .mempool
            .add_transaction(tx.clone(), None)
            .unwrap();
    }
    assert_eq!(mempool_task.mempool.num_transactions(), 2);

    let evicted_hash: Blake2bHash = txs[1].hash();
    assert!(mempool_task.mempool.is_filtered(&evicted_hash));

    let upgrade_block = next_election_block_with_version(&producer, &blockchain, upgrade_version);

    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), upgrade_block),
        Ok(PushResult::Extended)
    );

    loop {
        let event = timeout(Duration::from_secs(5), mempool_task.next())
            .await
            .unwrap()
            .unwrap();

        match BlockchainEvent::from(event) {
            BlockchainEvent::Extended(_) | BlockchainEvent::EpochFinalized(_) => {}
            BlockchainEvent::ProtocolUpgrade(_, version) => {
                assert_eq!(version, upgrade_version);
                assert_eq!(mempool_task.mempool.protocol_version(), upgrade_version);
                break;
            }
            other => panic!("unexpected event while waiting for protocol upgrade: {other:?}"),
        }
    }

    let remaining_hashes = mempool_task.mempool.get_transaction_hashes();
    assert_eq!(remaining_hashes.len(), 2);
    assert!(remaining_hashes.contains(&txs[0].hash::<Blake2bHash>()));
    assert!(remaining_hashes.contains(&evicted_hash));
    assert!(mempool_task
        .mempool
        .contains_transaction_by_hash(&txs[0].hash::<Blake2bHash>()));
    assert!(mempool_task
        .mempool
        .contains_transaction_by_hash(&evicted_hash));
    assert!(mempool_task.mempool.is_filtered(&evicted_hash));
}
