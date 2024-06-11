use std::{sync::Arc, task::Poll};

use futures::{poll, Stream, StreamExt};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{sync::syncer_proxy::SyncerProxy, Consensus};
use nimiq_genesis::NetworkId;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{Network, Topic};
use nimiq_network_mock::MockHub;
use nimiq_primitives::policy::Policy;
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[wasm_bindgen_test]
pub async fn it_can_initialize_with_mock_network() {
    let mut hub = MockHub::new();

    let mock_network = Arc::new(hub.new_network());

    let blockchain = Arc::new(RwLock::new(LightBlockchain::new(NetworkId::DevAlbatross)));

    let blockchain_proxy = BlockchainProxy::from(&blockchain);

    let zkp_component =
        ZKPComponent::new(blockchain_proxy.clone(), Arc::clone(&mock_network), None).await;

    let bls_cache = Arc::new(Mutex::new(PublicKeyCache::new(
        Policy::BLS_CACHE_MAX_CAPACITY,
    )));

    let network_events = mock_network.subscribe_events();

    let syncer = SyncerProxy::new_light(
        blockchain_proxy.clone(),
        Arc::clone(&mock_network),
        bls_cache,
        zkp_component.proxy(),
        network_events,
    )
    .await;

    // Initialize consensus
    let consensus = Consensus::new(
        blockchain_proxy.clone(),
        Arc::clone(&mock_network),
        syncer,
        3,
        zkp_component.proxy(),
    );

    if let Poll::Ready(_) = poll!(consensus) {
        panic!("Consensus should not be ready");
    }
}

// The following test is an adaptation of the mocking network test for wasm
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct TestRecord {
    x: i32,
}

pub struct TestTopic;

impl Topic for TestTopic {
    type Item = TestRecord;

    const BUFFER_SIZE: usize = 8;
    const NAME: &'static str = "test-wasm";
    const VALIDATE: bool = false;
}

fn consume_stream<T: std::fmt::Debug>(mut stream: impl Stream<Item = T> + Unpin + Send + 'static) {
    spawn_local(async move { while stream.next().await.is_some() {} });
}

#[wasm_bindgen_test]
async fn gossipsub_web_env() {
    let mut hub = MockHub::new();
    let net1 = hub.new_network();
    let net2 = hub.new_network();
    net1.dial_mock(&net2);

    let test_message = TestRecord { x: 42 };

    let mut messages = net1.subscribe::<TestTopic>().await.unwrap();
    consume_stream(net2.subscribe::<TestTopic>().await.unwrap());

    net2.publish::<TestTopic>(test_message.clone())
        .await
        .unwrap();

    let (received_message, _peer) = messages.next().await.unwrap();
    log::info!("Received Gossipsub message: {:?}", received_message);

    assert_eq!(received_message, test_message);
}
