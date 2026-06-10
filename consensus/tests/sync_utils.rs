use std::{
    ops::Add,
    sync::{atomic::AtomicU32, Arc},
};

use futures::{future, StreamExt};
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, Direction, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{
    consensus::Consensus,
    messages::{BlockBodyTopic, BlockHeaderMessage, BlockHeaderTopic},
    sync::{sync_interface::MacroSyncReturn, syncer_proxy::SyncerProxy},
    BlsCache,
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis::NetworkId;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::{
    blockchain::{
        fill_micro_blocks_with_txns, produce_macro_blocks_with_txns,
        signal_next_protocol_version_via_tx, signing_key, voting_key,
    },
    test_custom_block::{next_macro_block, BlockConfig},
    test_network::TestNetwork,
};
use nimiq_utils::{spawn, time::OffsetTime};
use parking_lot::{Mutex, RwLock};
use tokio::time::{timeout, Duration};

#[allow(dead_code)]
#[derive(PartialEq)]
pub enum SyncMode {
    History,
    Full,
    Light,
    Pico,
}

async fn syncer(
    sync_mode: &SyncMode,
    network: &Arc<Network>,
    blockchain: &BlockchainProxy,
) -> SyncerProxy<Network> {
    match sync_mode {
        SyncMode::History => {
            SyncerProxy::new_history(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(BlsCache::new_test())),
                network.subscribe_events(),
            )
            .await
        }
        SyncMode::Full => {
            SyncerProxy::new_full(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(BlsCache::new_test())),
                network.subscribe_events(),
                0,
                Arc::new(AtomicU32::new(0)),
            )
            .await
        }
        SyncMode::Light => {
            SyncerProxy::new_light(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(BlsCache::new_test())),
                network.subscribe_events(),
            )
            .await
        }
        SyncMode::Pico => {
            SyncerProxy::new_pico(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(BlsCache::new_test())),
                network.subscribe_events(),
            )
            .await
        }
    }
}

pub async fn sync_two_peers(
    num_batches_macro_sync: usize,
    num_batches_live_sync: usize,
    sync_mode: SyncMode,
) {
    let hub = MockHub::default();
    let mut networks = vec![];

    // Setup first peer.
    let env1 = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(
            env1.clone(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    // Produce the blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(&producer, &blockchain1, num_batches_macro_sync, 1, 2);

    let net1: Arc<Network> = TestNetwork::build_network(
        num_batches_macro_sync as u64 * 10,
        Default::default(),
        &mut Some(hub),
    )
    .await;
    networks.push(Arc::clone(&net1));
    let blockchain1_proxy = BlockchainProxy::from(&blockchain1);
    let syncer1 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        Arc::new(Mutex::new(BlsCache::new_test())),
        net1.subscribe_events(),
    )
    .await;
    let consensus1 = Consensus::from_network(
        BlockchainProxy::from(&blockchain1),
        Arc::clone(&net1),
        syncer1,
    );

    // Setup second peer (not synced yet).
    let time = Arc::new(OffsetTime::new());
    let env2 = MdbxDatabase::new_volatile(Default::default()).unwrap();

    let blockchain2_proxy = match sync_mode {
        SyncMode::History | SyncMode::Full => {
            let blockchain2 = Arc::new(RwLock::new(
                Blockchain::new(
                    env2.clone(),
                    BlockchainConfig::default(),
                    NetworkId::UnitAlbatross,
                    time,
                )
                .unwrap(),
            ));
            BlockchainProxy::from(blockchain2)
        }
        SyncMode::Light => {
            let blockchain2 = Arc::new(RwLock::new(LightBlockchain::new(NetworkId::UnitAlbatross)));
            BlockchainProxy::from(blockchain2)
        }
        SyncMode::Pico => {
            let blockchain2 = Arc::new(RwLock::new(LightBlockchain::new(NetworkId::UnitAlbatross)));
            BlockchainProxy::from(blockchain2)
        }
    };

    let net2: Arc<Network> = TestNetwork::build_network(
        num_batches_macro_sync as u64 * 10 + 1,
        Default::default(),
        &mut Some(MockHub::default()),
    )
    .await;
    networks.push(Arc::clone(&net2));

    let mut syncer2 = syncer(&sync_mode, &net2, &blockchain2_proxy).await;

    Network::connect_networks(&networks, num_batches_macro_sync as u64 * 10 + 1).await;

    let macro_sync_result = match syncer2 {
        SyncerProxy::History(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            macro_sync_result
        }
        SyncerProxy::Light(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            macro_sync_result
        }
        SyncerProxy::Full(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            macro_sync_result
        }
        SyncerProxy::Pico(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            macro_sync_result
        }
    };
    log::debug!("Macro sync result {:?}", macro_sync_result);
    assert_eq!(
        macro_sync_result,
        Some(MacroSyncReturn::Good(net1.get_local_peer_id()))
    );

    let consensus2 = Consensus::new(
        blockchain2_proxy.clone(),
        Arc::clone(&net2),
        syncer2,
        1,
        Arc::new(AtomicU32::new(0)),
    );
    let consensus2_proxy = consensus2.proxy();
    let events = blockchain2_proxy.read().notifier_as_stream();
    let mut events = events.filter(|event| {
        future::ready(matches!(
            event,
            BlockchainEvent::Finalized(_) | BlockchainEvent::EpochFinalized(_)
        ))
    });
    let mut consensus_events = consensus2_proxy.subscribe_events();
    spawn(consensus2);

    for _ in 0..num_batches_live_sync {
        let start_block_hash = blockchain1_proxy.read().head_hash();
        produce_macro_blocks_with_txns(&producer, &Arc::clone(&blockchain1), 1, 4, 2);
        let blocks = blockchain1_proxy
            .read()
            .get_blocks(
                &start_block_hash,
                Policy::blocks_per_batch(),
                true,
                Direction::Forward,
            )
            .unwrap();

        for block in blocks {
            let (header, body) = BlockHeaderMessage::split_block(block);
            _ = net1.publish::<BlockHeaderTopic>(header).await;
            if sync_mode == SyncMode::History || sync_mode == SyncMode::Full {
                _ = net1.publish::<BlockBodyTopic>(body).await;
            }
        }
        let sync_result = events.next().await;
        assert!(sync_result.is_some());
    }
    let consensus1_proxy = consensus1.proxy();
    _ = consensus_events.next().await;
    assert!(consensus2_proxy.is_established());
    assert_eq!(
        blockchain2_proxy.read().election_head().block_number(),
        consensus1_proxy
            .blockchain
            .read()
            .election_head()
            .block_number(),
    );
    assert_eq!(
        blockchain2_proxy.read().election_head_hash(),
        consensus1_proxy.blockchain.read().election_head_hash(),
    );
    assert_eq!(
        blockchain2_proxy.read().macro_head_hash(),
        consensus1_proxy.blockchain.read().macro_head_hash(),
    );

    match blockchain2_proxy {
        BlockchainProxy::Full(blockchain) => {
            assert!(blockchain.read().accounts_complete());
        }
        BlockchainProxy::Light(_) => {}
    }
}

pub async fn sync_two_peers_across_protocol_upgrade(sync_mode: SyncMode) {
    let hub = MockHub::default();
    let mut networks = vec![];

    let env1 = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(
            env1,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(
        &producer,
        &blockchain1,
        (Policy::batches_per_epoch() - 1) as usize,
        1,
        2,
    );
    let b1_init_version = blockchain1.read().protocol_version();

    let upgrade_version = signal_next_protocol_version_via_tx(&producer, &blockchain1);
    fill_micro_blocks_with_txns(&producer, &blockchain1, 1, 2);
    let upgrade_block = {
        let blockchain = blockchain1.read();
        next_macro_block(
            &producer.signing_key,
            &producer.voting_key,
            &blockchain,
            &BlockConfig {
                version: Some(upgrade_version),
                ..Default::default()
            },
        )
    };
    let upgrade_hash = upgrade_block.hash();
    assert_eq!(
        Blockchain::push(blockchain1.upgradable_read(), upgrade_block),
        Ok(PushResult::Extended)
    );

    let net1: Arc<Network> =
        TestNetwork::build_network(40, Default::default(), &mut Some(hub)).await;
    networks.push(Arc::clone(&net1));
    let blockchain1_proxy = BlockchainProxy::from(&blockchain1);
    let syncer1 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        Arc::new(Mutex::new(BlsCache::new_test())),
        net1.subscribe_events(),
    )
    .await;
    let _consensus1 = Consensus::from_network(blockchain1_proxy, Arc::clone(&net1), syncer1);

    let time = Arc::new(OffsetTime::new());
    let env2 = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let blockchain2_proxy = match sync_mode {
        SyncMode::History | SyncMode::Full => {
            let blockchain2 = Arc::new(RwLock::new(
                Blockchain::new(
                    env2,
                    BlockchainConfig::default(),
                    NetworkId::UnitAlbatross,
                    time,
                )
                .unwrap(),
            ));
            BlockchainProxy::from(blockchain2)
        }
        SyncMode::Light | SyncMode::Pico => {
            let blockchain2 = Arc::new(RwLock::new(LightBlockchain::new(NetworkId::UnitAlbatross)));
            BlockchainProxy::from(blockchain2)
        }
    };
    let b2_init_version = blockchain2_proxy.read().protocol_version();

    let events = blockchain2_proxy.read().notifier_as_stream();
    let mut upgrade_events = events
        .filter(|event| future::ready(matches!(event, BlockchainEvent::ProtocolUpgrade(_, _))));

    let net2: Arc<Network> =
        TestNetwork::build_network(41, Default::default(), &mut Some(MockHub::default())).await;
    networks.push(Arc::clone(&net2));

    let mut syncer2 = syncer(&sync_mode, &net2, &blockchain2_proxy).await;

    Network::connect_networks(&networks, 41).await;

    let macro_sync_result = match syncer2 {
        SyncerProxy::History(ref mut syncer) => syncer.macro_sync.next().await,
        SyncerProxy::Light(ref mut syncer) => syncer.macro_sync.next().await,
        SyncerProxy::Full(ref mut syncer) => syncer.macro_sync.next().await,
        SyncerProxy::Pico(ref mut syncer) => syncer.macro_sync.next().await,
    };

    assert_eq!(
        macro_sync_result,
        Some(MacroSyncReturn::Good(net1.get_local_peer_id()))
    );

    match timeout(Duration::from_secs(10), upgrade_events.next()).await {
        Ok(Some(BlockchainEvent::ProtocolUpgrade(block_hash, version))) => {
            assert_eq!(block_hash, upgrade_hash);
            assert_eq!(version, upgrade_version);
        }
        Ok(event) => panic!("expected ProtocolUpgrade event, got {event:?}"),
        Err(_) => panic!("timed out waiting for ProtocolUpgrade event"),
    }
    assert_eq!(b1_init_version, b2_init_version);
    assert_eq!(b2_init_version.add(1), upgrade_version);
    assert_eq!(blockchain2_proxy.read().election_head_hash(), upgrade_hash);
    assert_eq!(blockchain2_proxy.read().protocol_version(), upgrade_version);
}
