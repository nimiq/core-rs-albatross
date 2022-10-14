use std::sync::Arc;

use futures::StreamExt;

use nimiq_blockchain::Blockchain;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_nano_zkp::NanoZKP;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_zkp_prover::proof_component as ZKProofComponent;
use nimiq_zkp_prover::zkp_requests::ZKPRequests;
use nimiq_zkp_prover::ZKPComponent;
use parking_lot::RwLock;

use nimiq_utils::time::OffsetTime;

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ))
}

#[test(tokio::test)]
async fn peers_dont_reply_with_outdated_proof() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    let network3 = Arc::new(hub.new_network());
    network.dial_address(network3.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let _zkp_prover2 = ZKPComponent::new(
        Arc::clone(&blockchain),
        Arc::clone(&network2),
        false,
        VolatileEnvironment::new(10).unwrap(),
    )
    .await;

    let _zkp_prover3 = ZKPComponent::new(
        Arc::clone(&blockchain),
        Arc::clone(&network3),
        false,
        VolatileEnvironment::new(10).unwrap(),
    )
    .await;

    let mut zkp_requests = ZKPRequests::new(Arc::clone(&network));

    // Trigger zkp requests
    zkp_requests.request_zkps(network.get_peers(), 0);

    for _ in 0..2 {
        assert!(
            zkp_requests.next().await.is_none(),
            "Peer sent a proof when it should have abstained because of having an outdated proof"
        );
    }
}

#[test(tokio::test)]
#[ignore]
async fn peers_reply_with_valid_proof() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let network2 = Arc::new(hub.new_network());
    let network3 = Arc::new(hub.new_network());
    network.dial_address(network3.address()).await.unwrap();
    network.dial_address(network2.address()).await.unwrap();

    let _zkp_prover2 = ZKPComponent::new(
        Arc::clone(&blockchain),
        Arc::clone(&network2),
        false,
        VolatileEnvironment::new(10).unwrap(),
    )
    .await;

    let _zkp_prover3 = ZKPComponent::new(
        Arc::clone(&blockchain),
        Arc::clone(&network3),
        false,
        VolatileEnvironment::new(10).unwrap(),
    )
    .await;

    let mut zkp_requests = ZKPRequests::new(Arc::clone(&network));

    // ITODO set valid proof into the 2 components and request proofs

    // Trigger zkp requests
    zkp_requests.request_zkps(network.get_peers(), 0);

    for _ in 0..2 {
        let proof = zkp_requests.next().await;
        assert!(proof.is_some(), "Peer sent a new proof");
        assert!(
            ZKProofComponent::validate_proof(&blockchain, proof.unwrap().1),
            "Peer sent a new proof"
        );
    }
}
