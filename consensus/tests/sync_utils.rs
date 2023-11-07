use std::sync::Arc;

use futures::{future, StreamExt};
use nimiq_block::{BlockHeaderTopic, BlockTopic};
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, Direction};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{
    consensus::Consensus,
    sync::{
        syncer::{MacroSyncReturn, ValidityWindowSyncReturn},
        syncer_proxy::SyncerProxy,
    },
};
use nimiq_database::volatile::VolatileDatabase;
use nimiq_genesis::NetworkId;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks_with_txns, signing_key, voting_key},
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
    test_network::TestNetwork,
};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};
use tokio::spawn;

#[allow(dead_code)]
pub enum SyncMode {
    History,
    Full,
    Light,
}

async fn syncer(
    sync_mode: &SyncMode,
    network: &Arc<Network>,
    blockchain: &BlockchainProxy,
    zkp_prover: &ZKPComponent<Network>,
) -> SyncerProxy<Network> {
    match sync_mode {
        SyncMode::History => {
            SyncerProxy::new_history(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(PublicKeyCache::new(
                    TESTING_BLS_CACHE_MAX_CAPACITY,
                ))),
                network.subscribe_events(),
            )
            .await
        }
        SyncMode::Full => {
            SyncerProxy::new_full(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(PublicKeyCache::new(
                    TESTING_BLS_CACHE_MAX_CAPACITY,
                ))),
                zkp_prover.proxy(),
                network.subscribe_events(),
                0,
            )
            .await
        }
        SyncMode::Light => {
            SyncerProxy::new_light(
                blockchain.clone(),
                Arc::clone(network),
                Arc::new(Mutex::new(PublicKeyCache::new(
                    TESTING_BLS_CACHE_MAX_CAPACITY,
                ))),
                zkp_prover.proxy(),
                network.subscribe_events(),
                Box::new(|fut| {
                    tokio::spawn(fut);
                }),
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
    let env1 = VolatileDatabase::new(20).unwrap();
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
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
        net1.subscribe_events(),
    )
    .await;
    let zkp_prover1 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain1),
        Arc::clone(&net1),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        None,
    )
    .await;
    let consensus1 = Consensus::from_network(
        BlockchainProxy::from(&blockchain1),
        Arc::clone(&net1),
        syncer1,
        zkp_prover1.proxy(),
    );

    // Setup second peer (not synced yet).
    let time = Arc::new(OffsetTime::new());
    let env2 = VolatileDatabase::new(20).unwrap();

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
    };

    let net2: Arc<Network> = TestNetwork::build_network(
        num_batches_macro_sync as u64 * 10 + 1,
        Default::default(),
        &mut Some(MockHub::default()),
    )
    .await;
    networks.push(Arc::clone(&net2));

    let zkp_prover2 = ZKPComponent::new(
        blockchain2_proxy.clone(),
        Arc::clone(&net2),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        None,
    )
    .await;

    let mut syncer2 = syncer(&sync_mode, &net2, &blockchain2_proxy, &zkp_prover2).await;

    Network::connect_networks(&networks, num_batches_macro_sync as u64 * 10 + 1).await;
    let zkp_prover2_proxy = zkp_prover2.proxy();
    spawn(zkp_prover2);

    let macro_sync_result = match syncer2 {
        SyncerProxy::History(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_validity_window_sync(net1.get_local_peer_id());
            macro_sync_result
        }
        SyncerProxy::Light(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_validity_window_sync(net1.get_local_peer_id());
            macro_sync_result
        }
        SyncerProxy::Full(ref mut syncer) => {
            let macro_sync_result = syncer.macro_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_validity_window_sync(net1.get_local_peer_id());
            macro_sync_result
        }
    };

    log::debug!("Macro sync result {:?}", macro_sync_result);
    assert_eq!(
        macro_sync_result,
        Some(MacroSyncReturn::Good(net1.get_local_peer_id()))
    );

    let validity_sync_result = match syncer2 {
        SyncerProxy::History(ref mut syncer) => {
            let validity_sync_result = syncer.validity_window_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            validity_sync_result
        }
        SyncerProxy::Light(ref mut syncer) => {
            let validity_sync_result = syncer.validity_window_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            validity_sync_result
        }
        SyncerProxy::Full(ref mut syncer) => {
            let validity_sync_result = syncer.validity_window_sync.next().await;
            // Now we move the syncing peer to the live sync.
            syncer.move_peer_into_live_sync(net1.get_local_peer_id());
            validity_sync_result
        }
    };
    log::debug!("validity sync result {:?}", validity_sync_result);
    assert_eq!(
        validity_sync_result,
        Some(ValidityWindowSyncReturn::Good(net1.get_local_peer_id()))
    );

    // TODO check the size of the history store after the validity window sync and verify it has everything we need

    let consensus2 = Consensus::new(
        blockchain2_proxy.clone(),
        Arc::clone(&net2),
        syncer2,
        1,
        zkp_prover2_proxy,
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
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
            match sync_mode {
                SyncMode::Light => {
                    _ = net1.publish::<BlockHeaderTopic>(block).await;
                }
                SyncMode::History | SyncMode::Full => {
                    _ = net1.publish::<BlockTopic>(block.clone()).await;
                }
            };
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
        blockchain2_proxy.read().macro_head(),
        consensus1_proxy.blockchain.read().macro_head(),
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
