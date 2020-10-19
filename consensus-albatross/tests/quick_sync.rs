use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::Block;
use nimiq_block_production_albatross::{test_utils::*, BlockProducer};
use nimiq_blockchain_albatross::{Blockchain, PushResult};
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_consensus_albatross::consensus::Consensus;
use nimiq_consensus_albatross::messages::RequestBlockHashesFilter;
use nimiq_consensus_albatross::sync::QuickSync;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_mock::network::MockNetwork;
use nimiq_primitives::policy;

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

// FIXME: Enable this test when history root refactor is ready.
// #[tokio::test]
async fn peers_can_sync() {
    // Setup first peer.
    let env1 = VolatileEnvironment::new(10).unwrap();
    let blockchain1 = Arc::new(Blockchain::new(env1.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool1 = Mempool::new(Arc::clone(&blockchain1), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(
        Arc::clone(&blockchain1),
        Arc::clone(&mempool1),
        keypair.clone(),
    );

    while policy::epoch_at(blockchain1.block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain1);

        let (proposal, extrinsics) = producer.next_macro_block_proposal(
            blockchain1.time.now() + blockchain1.block_number() as u64 * 1000,
            0u32,
            None,
            vec![0x42],
        );

        let block = sign_macro_block(&keypair, proposal, Some(extrinsics));
        assert_eq!(
            blockchain1.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }

    let net1 = Arc::new(MockNetwork::new(1));
    let sync1 = QuickSync::default();
    let consensus1 = Consensus::new(env1, blockchain1, mempool1, Arc::clone(&net1), sync1).unwrap();

    // Setup second peer (not synced yet).
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain2 = Arc::new(Blockchain::new(env2.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool2 = Mempool::new(Arc::clone(&blockchain2), MempoolConfig::default());

    let net2 = Arc::new(MockNetwork::new(2));
    let sync2 = QuickSync::default();
    let consensus2 = Consensus::new(env2, blockchain2, mempool2, Arc::clone(&net2), sync2).unwrap();

    // Connect the two peers.
    net1.connect(&net2);
    // Then wait for connection to be established.
    let mut stream = consensus2.subscribe_events();
    stream.recv().await;

    assert_eq!(consensus2.num_agents(), 1);

    // Test ingredients:
    // Request hashes
    let agent = Arc::clone(consensus2.agents().values().next().unwrap());
    let hashes = agent
        .request_block_hashes(
            vec![consensus2.blockchain.head_hash()],
            2,
            RequestBlockHashesFilter::ElectionOnly,
        )
        .await
        .expect("Should yield hashes");
    assert_eq!(hashes.hashes.len(), 1);
    assert_eq!(hashes.hashes[0], consensus1.blockchain.election_head_hash());

    // Request epoch
    let epoch = agent
        .request_epoch(consensus1.blockchain.election_head_hash())
        .await
        .expect("Should yield epoch");
    assert_eq!(epoch.history_len, 0);
    assert_eq!(
        epoch.block.hash(),
        consensus1.blockchain.election_head_hash()
    );

    let sync_result = Consensus::sync_blockchain(Arc::downgrade(&consensus2)).await;

    assert!(sync_result.is_ok());
    assert_eq!(
        consensus2.blockchain.election_head_hash(),
        consensus1.blockchain.election_head_hash(),
    );

    // Setup third peer (not synced yet).
    let env3 = VolatileEnvironment::new(10).unwrap();
    let blockchain3 = Arc::new(Blockchain::new(env3.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool3 = Mempool::new(Arc::clone(&blockchain3), MempoolConfig::default());

    let net3 = Arc::new(MockNetwork::new(3));
    let sync3 = QuickSync::default();
    let consensus3 = Consensus::new(env3, blockchain3, mempool3, Arc::clone(&net3), sync3).unwrap();

    // Third peer has two micro blocks that need to be reverted.
    for i in 1..4 {
        consensus3
            .blockchain
            .push(
                consensus1
                    .blockchain
                    .chain_store
                    .get_block_at(i, true, None)
                    .unwrap(),
            )
            .unwrap();
    }

    // Connect the new peer with macro synced peer.
    net3.connect(&net2);
    // Then wait for connection to be established.
    let mut stream = consensus3.subscribe_events();
    stream.recv().await;

    assert_eq!(consensus3.num_agents(), 1);

    // Test ingredients:
    // Request hashes
    let agent = Arc::clone(consensus3.agents().values().next().unwrap());
    let hashes = agent
        .request_block_hashes(
            vec![consensus3.blockchain.head_hash()],
            2,
            RequestBlockHashesFilter::ElectionOnly,
        )
        .await
        .expect("Should yield hashes");
    assert_eq!(hashes.hashes.len(), 1);
    assert_eq!(hashes.hashes[0], consensus2.blockchain.election_head_hash());

    // Request epoch
    let epoch = agent
        .request_epoch(consensus2.blockchain.election_head_hash())
        .await
        .expect("Should yield epoch");
    assert_eq!(epoch.history_len, 0);
    assert_eq!(
        epoch.block.hash(),
        consensus2.blockchain.election_head_hash()
    );

    let sync_result = Consensus::sync_blockchain(Arc::downgrade(&consensus3)).await;

    assert!(sync_result.is_ok());
    assert_eq!(
        consensus3.blockchain.election_head_hash(),
        consensus1.blockchain.election_head_hash()
    );
}
