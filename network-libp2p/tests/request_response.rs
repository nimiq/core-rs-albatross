use std::{sync::Arc, time::Duration};

use futures::{future::join_all, StreamExt};
use libp2p::{
    core::multiaddr::{multiaddr, Multiaddr},
    gossipsub::GossipsubConfigBuilder,
    identity::Keypair,
    swarm::KeepAlive,
};
use rand::{thread_rng, Rng};

use beserial::{Deserialize, Serialize};
use beserial_derive::{Deserialize, Serialize};
use nimiq_network_interface::{
    network::Network as NetworkInterface,
    prelude::{InboundRequestError, NetworkEvent, OutboundRequestError, Request, RequestError},
};
use nimiq_network_libp2p::{
    discovery::{
        behaviour::DiscoveryConfig,
        peer_contacts::{PeerContact, Protocols, Services},
    },
    Config, Network, PeerId,
};
use nimiq_test_log::test;
use nimiq_utils::time::OffsetTime;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestRequest {
    request: u64,
}
impl Request for TestRequest {
    const TYPE_ID: u16 = 42;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse {
    response: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse2 {
    response: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse3 {
    response: u32,
}

#[derive(Clone, Debug)]
struct TestNetwork {}

impl TestNetwork {
    async fn create_connected_networks() -> (Network, Network) {
        log::debug!("creating connected test networks:");
        let addr1 = multiaddr![Memory(thread_rng().gen::<u64>())];
        let addr2 = multiaddr![Memory(thread_rng().gen::<u64>())];

        let net1 = Network::new(Arc::new(OffsetTime::new()), network_config(addr1.clone())).await;
        net1.listen_on(vec![addr1.clone()]).await;

        let net2 = Network::new(Arc::new(OffsetTime::new()), network_config(addr2.clone())).await;
        net2.listen_on(vec![addr2.clone()]).await;

        log::debug!(address = ?addr1, peer_id = ?net1.get_local_peer_id(), "Network 1");
        log::debug!(address = ?addr2, peer_id = ?net2.get_local_peer_id(), "Network 2");

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        log::debug!("dialing peer 1 from peer 2...");
        net2.dial_address(addr1).await.unwrap();

        log::debug!("waiting for join events");

        let event1 = events1.next().await.unwrap().unwrap();
        log::trace!(event1 = ?event1);
        assert_peer_joined(&event1, &net2.get_local_peer_id());

        let event2 = events2.next().await.unwrap().unwrap();
        log::trace!(event2 = ?event2);
        assert_peer_joined(&event2, &net1.get_local_peer_id());

        (net1, net2)
    }
}

fn network_config(address: Multiaddr) -> Config {
    let keypair = Keypair::generate_ed25519();

    let mut peer_contact = PeerContact {
        addresses: vec![address],
        public_key: keypair.public(),
        services: Services::all(),
        timestamp: None,
    };
    peer_contact.set_current_time();

    let gossipsub = GossipsubConfigBuilder::default()
        .validation_mode(libp2p::gossipsub::ValidationMode::Permissive)
        .build()
        .expect("Invalid Gossipsub config");

    Config {
        keypair,
        peer_contact,
        seeds: Vec::new(),
        discovery: DiscoveryConfig {
            genesis_hash: Default::default(),
            update_interval: Duration::from_secs(60),
            min_recv_update_interval: Duration::from_secs(30),
            update_limit: 64,
            protocols_filter: Protocols::all(),
            services_filter: Services::all(),
            min_send_update_interval: Duration::from_secs(30),
            house_keeping_interval: Duration::from_secs(60),
            keep_alive: KeepAlive::Yes,
        },
        kademlia: Default::default(),
        gossipsub,
        memory_transport: true,
    }
}

fn assert_peer_joined(event: &NetworkEvent<PeerId>, wanted_peer_id: &PeerId) {
    if let NetworkEvent::PeerJoined(peer_id) = event {
        assert_eq!(peer_id, wanted_peer_id);
    } else {
        panic!("Event is not a NetworkEvent::PeerJoined: {:?}", event);
    }
}

// Test that we can send a request and correctly receive the response given a proper
// request listener is replying in the peer specified
#[test(tokio::test)]
async fn test_valid_request_valid_response() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };
    let test_response = TestResponse { response: 43 };

    // Subscribe for receiving requests
    let mut requests = net1.receive_requests::<TestRequest>();

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request...");

    // Send the request and get future for the response
    let response = net2
        .request::<TestRequest, TestResponse>(test_request.clone(), net1.get_local_peer_id())
        .await
        .unwrap();

    log::info!("Waiting for Request message...");
    let (received_request, request_id, peer_id) = requests.next().await.unwrap();
    log::info!(
        "Received request {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_request
    );

    assert_eq!(received_request, test_request);

    // Respond the request
    assert!(net1
        .respond(request_id, test_response.clone())
        .await
        .is_ok());

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Check the received response
    let (received_response, request_id, peer_id) = response.await;
    log::info!(
        "Received response {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_response
    );

    match received_response {
        Ok(response) => {
            assert_eq!(response, test_response);
        }
        Err(e) => assert!(false, "Response received with error: {:?}", e),
    };
}

// Test that we can send multiple requests and correctly receive the responses given a proper
// request listener is replying in the peer specified
#[test(tokio::test(flavor = "multi_thread", worker_threads = 10))]
async fn test_multiple_valid_requests_valid_responses() {
    let num_requests = 100;
    let mut response_futures = vec![];
    let (net1, net2) = TestNetwork::create_connected_networks().await;
    let peer_id_net1 = net1.get_local_peer_id();
    let net1 = Arc::new(net1);

    let test_request = TestRequest { request: 42 };
    let test_response = TestResponse { response: 43 };
    let test_response_2 = test_response.clone();

    // Subscribe for receiving requests and respond to them in a future
    let request_stream = net1.receive_requests::<TestRequest>();
    let request_listener_future =
        request_stream.for_each(move |(_request, request_id, _peer_id)| {
            let test_response = test_response.clone();
            let net1 = Arc::clone(&net1);
            async move {
                let result = net1.respond(request_id, test_response.clone()).await;
                assert!(result.is_ok());
            }
        });

    // Spawn the request listener future
    tokio::spawn(request_listener_future);

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending requests...");

    for _ in 0..num_requests {
        // Send the request and get future for the response
        let response_future = net2
            .request::<TestRequest, TestResponse>(test_request.clone(), peer_id_net1)
            .await
            .unwrap();

        response_futures.push(response_future);
    }

    log::info!("Waiting for requests to arrive and check them");
    let responses = join_all(response_futures).await;
    for (received_response, _request_id, _peer_id) in responses {
        match received_response {
            Ok(response) => {
                assert_eq!(response, test_response_2);
            }
            Err(e) => assert!(false, "Response received with error: {:?}", e),
        };
    }
}

// Test that we can send a request and receive a `DeSerializationError` if the peer replied with
// a message with the expected type ID but refers to a completely different type in reality
#[test(tokio::test)]
async fn test_valid_request_incorrect_response() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };
    let incorrect_response = TestResponse3 { response: 43 };

    // Subscribe for receiving requests
    let mut requests = net1.receive_requests::<TestRequest>();

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request...");

    // Send the request and get future for the response
    let response = net2
        .request::<TestRequest, TestResponse>(test_request.clone(), net1.get_local_peer_id())
        .await
        .unwrap();

    log::info!("Waiting for Request message...");
    let (received_request, request_id, peer_id) = requests.next().await.unwrap();
    log::info!(
        "Received request {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_request
    );

    assert_eq!(received_request, test_request);

    // Respond the request
    assert!(net1
        .respond(request_id, incorrect_response.clone())
        .await
        .is_ok());

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Check the received response
    let (received_response, request_id, peer_id) = response.await;
    log::info!(
        "Received response {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_response
    );

    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {:?}", response);
        }
        Err(e) => assert_eq!(
            e,
            RequestError::InboundRequest(InboundRequestError::DeSerializationError),
            "Response received with error: {:?}",
            e
        ),
    };
}

// Test that we can send a request and receive a timeout response if no response is
// provided.
#[test(tokio::test)]
async fn test_valid_request_no_response() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };

    // Subscribe for receiving requests
    let mut requests = net1.receive_requests::<TestRequest>();

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request...");

    // Send the request and get future for the response
    let response = net2
        .request::<TestRequest, TestResponse>(test_request.clone(), net1.get_local_peer_id())
        .await
        .unwrap();

    log::info!("Waiting for Request message...");
    let (received_request, request_id, peer_id) = requests.next().await.unwrap();
    log::info!(
        "Received request {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_request
    );

    assert_eq!(received_request, test_request);

    // Don't respond the request: It should timeout and send the error to the request
    // Check the received response
    let (received_response, request_id, peer_id) = response.await;
    log::info!(
        "Received response {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_response
    );

    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {:?}", response)
        }
        Err(e) => assert_eq!(
            e,
            RequestError::OutboundRequest(OutboundRequestError::Timeout)
        ),
    };
}

// Test that we can send a request and receive a timeout response if no response is sent
// given that no receiver is registered for the requests being sent
#[test(tokio::test)]
async fn test_valid_request_no_response_no_receiver() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request...");

    // Send the request and get future for the response
    let response = net2
        .request::<TestRequest, TestResponse>(test_request.clone(), net1.get_local_peer_id())
        .await
        .unwrap();

    // Don't respond the request: It should timeout and send the error to the request
    // Check the received response
    let (received_response, request_id, peer_id) = response.await;
    log::info!(
        "Received response {:?} from peer {:?}: {:?}",
        request_id,
        peer_id,
        received_response
    );

    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {:?}", response)
        }
        Err(e) => assert_eq!(
            e,
            RequestError::OutboundRequest(OutboundRequestError::NoReceiver)
        ),
    };
}
