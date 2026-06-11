use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, PushResult};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_mempool::{config::MempoolConfig, verify::VerifyErr};
use nimiq_mempool_task::MempoolTask;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{
    account::AccountError,
    coin::Coin,
    networks::NetworkId,
    policy::{upgrades, Policy},
};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        fill_micro_blocks_with_txns, next_protocol_upgrade_block, produce_macro_blocks_with_txns,
        signing_key, validator_address, voting_key,
    },
    node::Node,
    test_rng,
    test_transaction::generate_accounts,
};
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
async fn mempool_task_transactions_evicted_after_protocol_upgrade() {
    let mut rng = test_rng(true);
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let mut genesis_builder = GenesisBuilder::default();
    let initial_protocol_version = upgrades::v3::STAKING_CHANGE_ADD_STAKE_POLICY - 1;
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    genesis_builder.with_genesis_block_number(Policy::genesis_block_number());

    let sender_accounts = generate_accounts(
        vec![Policy::MINIMUM_STAKE + 100, 100],
        &mut genesis_builder,
        true,
        &mut rng,
    );
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
    genesis_builder.with_genesis_staker(
        recipient_accounts[1].address.clone(),
        validator_address(),
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
        Coin::ZERO,
        None,
    );

    let mut genesis_info = genesis_builder.generate(env).unwrap();
    {
        let header = &mut genesis_info.block.unwrap_macro_ref_mut().header;
        header.version = initial_protocol_version;
        header.cached_hash = None;
    }
    let genesis_hash = genesis_info.block.hash();
    genesis_info
        .block
        .populate_cached_hash(genesis_hash.clone());
    genesis_info.hash = genesis_hash;
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
    assert_eq!(
        blockchain.read().protocol_version(),
        initial_protocol_version
    );
    fill_micro_blocks_with_txns(&producer, &blockchain, 0, 0);
    let (upgrade_block, _) = next_protocol_upgrade_block(&producer, &blockchain);

    let validity_start_height = blockchain.read().block_number() + 1;
    let valid_basic_tx = TransactionBuilder::new_basic(
        &sender_accounts[1].keypair,
        recipient_accounts[0].address.clone(),
        Coin::from_u64_unchecked(10),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let evicted_tx = TransactionBuilder::new_add_stake(
        &sender_accounts[0].keypair,
        recipient_accounts[1].address.clone(),
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE - 1),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let post_released_balance_tx = TransactionBuilder::new_basic(
        &sender_accounts[0].keypair,
        recipient_accounts[0].address.clone(),
        Coin::from_u64_unchecked(Policy::MINIMUM_STAKE),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    consensus.force_established();

    let mut mempool_task = MempoolTask::new(
        &consensus,
        Arc::clone(&blockchain),
        MempoolConfig::default(),
    );

    mempool_task
        .mempool
        .add_transaction(valid_basic_tx.clone(), None)
        .unwrap();
    mempool_task
        .mempool
        .add_transaction(evicted_tx.clone(), None)
        .unwrap();
    assert_eq!(mempool_task.mempool.num_transactions(), 2);

    let evicted_hash: Blake2bHash = evicted_tx.hash();
    let valid_basic_hash: Blake2bHash = valid_basic_tx.hash();
    let post_released_balance_hash: Blake2bHash = post_released_balance_tx.hash();
    let upgrade_block_hash = upgrade_block.hash();

    // This spend from the same sender only fails because the add-stake transaction still reserves
    // almost the entire sender balance in the mempool.
    assert!(matches!(
        mempool_task
            .mempool
            .add_transaction(post_released_balance_tx.clone(), None),
        Err(VerifyErr::InvalidAccount(
            AccountError::InsufficientFunds { .. }
        ))
    ));

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
            BlockchainEvent::Extended(block_hash) => {
                assert_eq!(block_hash, upgrade_block_hash);
                break;
            }
            BlockchainEvent::EpochFinalized(_) | BlockchainEvent::ProtocolUpgrade(..) => {}
            other => panic!("unexpected event while waiting for protocol upgrade: {other:?}"),
        }
    }

    // Once the protocol upgrade evicts the add-stake transaction, its reserved balance must be
    // released so this second spend becomes admissible.
    mempool_task
        .mempool
        .add_transaction(post_released_balance_tx.clone(), None)
        .unwrap();

    let remaining_hashes = mempool_task.mempool.get_transaction_hashes();
    assert_eq!(remaining_hashes.len(), 2);
    assert!(remaining_hashes.contains(&valid_basic_hash));
    assert!(remaining_hashes.contains(&post_released_balance_hash));
    assert!(!remaining_hashes.contains(&evicted_hash));
    assert!(mempool_task
        .mempool
        .contains_transaction_by_hash(&valid_basic_hash));
    assert!(mempool_task
        .mempool
        .contains_transaction_by_hash(&post_released_balance_hash));
    assert!(!mempool_task
        .mempool
        .contains_transaction_by_hash(&evicted_hash));
}
