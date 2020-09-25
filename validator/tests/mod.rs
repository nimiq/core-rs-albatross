use std::sync::Arc;
use std::time::Duration;

use futures::{future, StreamExt};

use beserial::Deserialize;
use nimiq_blockchain_albatross::Blockchain;
use nimiq_bls::KeyPair;
use nimiq_consensus_albatross::sync::QuickSync;
use nimiq_consensus_albatross::Consensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_mock::network::MockNetwork;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::validator::Validator;

const SIGNING_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

fn mock_consensus(peer_id: u32) -> Arc<Consensus<MockNetwork>> {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let network = Arc::new(MockNetwork::new(peer_id));
    let sync_protocol = QuickSync::default();
    Consensus::new(env, blockchain, mempool, network, sync_protocol).unwrap()
}

#[tokio::test]
async fn validator_can_create_micro_blocks() {
    let consensus1 = mock_consensus(1);
    let signing_key = KeyPair::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap();
    let validator = Validator::new(
        Arc::clone(&consensus1),
        Arc::clone(&consensus1.network), // TODO
        signing_key,
        None,
    );

    let consensus2 = mock_consensus(2);
    consensus2.network.connect(&consensus1.network);

    let mut events1 = consensus1.subscribe_events();
    events1.recv().await;

    Consensus::sync_blockchain(Arc::downgrade(&consensus1))
        .await
        .expect("Sync failed");

    assert_eq!(consensus1.established(), true);

    tokio::spawn(validator);

    let events1 = consensus1.blockchain.notifier.write().as_stream();
    events1.take(10).for_each(|_| future::ready(())).await;

    assert!(consensus1.blockchain.block_number() >= 10);
}
