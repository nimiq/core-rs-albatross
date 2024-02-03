use std::sync::Arc;

use futures::{future::join_all, StreamExt};
use libp2p::{
    core::multiaddr::{multiaddr, Multiaddr},
    gossipsub,
    identity::Keypair,
};
#[cfg(feature = "tokio-time")]
use nimiq_network_interface::network::CloseReason;
use nimiq_network_interface::{
    network::Network as NetworkInterface,
    peer_info::Services,
    request::{
        InboundRequestError, OutboundRequestError, Request, RequestCommon, RequestError,
        RequestMarker,
    },
};
use nimiq_network_libp2p::{
    discovery::{self, peer_contacts::PeerContact},
    Config, Network,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use rand::{thread_rng, Rng};
use tokio::time::Duration;
#[cfg(feature = "tokio-time")]
use tokio::time::Instant;

mod helper;

/// The max number of TestRequests per peer (used for regular tests only).
const MAX_REQUEST_RESPONSE_TEST_REQUEST: u32 = 1000;
/// The max number of TestRequests per peer. This is used exclusively for rate limiting testing.
const MAX_REQUEST_RESPONSE_STRESS_TEST_REQUEST: u32 = 2;
/// The range to restrict the responses to the requests on the testing of the network layer.
/// This is used exclusively for rate limiting testing.
const TEST_MAX_REQUEST_RESPONSE_STRESS_TEST_WINDOW: Duration = Duration::from_secs(100);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestRequest {
    request: u64,
}
impl RequestCommon for TestRequest {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 42;
    type Response = TestResponse;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_TEST_REQUEST;
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse {
    response: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestRequest2 {
    request: u64,
}
impl RequestCommon for TestRequest2 {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 42;
    type Response = TestResponse2;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_TEST_REQUEST;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse2 {
    response: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestRequest3 {
    request: u64,
}
impl RequestCommon for TestRequest3 {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 42;
    type Response = TestResponse3;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_TEST_REQUEST;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse3 {
    response: [u8; 8],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestRequest4 {
    request: u64,
}
impl RequestCommon for TestRequest4 {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 42;
    type Response = TestResponse4;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_STRESS_TEST_REQUEST;
    const TIME_WINDOW: Duration = TEST_MAX_REQUEST_RESPONSE_STRESS_TEST_WINDOW;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse4 {
    response: u32,
}

#[derive(Clone, Debug)]
struct TestNetwork {}

impl TestNetwork {
    async fn create_connected_networks() -> (Network, Network) {
        log::debug!("Creating connected test networks");
        let mut rng = thread_rng();
        let addr1 = multiaddr![Memory(rng.gen::<u64>())];
        let addr2 = multiaddr![Memory(rng.gen::<u64>())];

        let net1 = Network::new(
            network_config(addr1.clone()),
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        )
        .await;
        net1.listen_on(vec![addr1.clone()]).await;

        let net2 = Network::new(
            network_config(addr2.clone()),
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        )
        .await;
        net2.listen_on(vec![addr2.clone()]).await;

        log::debug!(address = %addr1, peer_id = %net1.get_local_peer_id(), "Network 1");
        log::debug!(address = %addr2, peer_id = %net2.get_local_peer_id(), "Network 2");

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        log::debug!("Dialing peer 1 from peer 2");
        net2.dial_address(addr1).await.unwrap();

        log::debug!("Waiting for join events");

        let event1 = helper::get_next_peer_event(&mut events1).await;
        log::trace!(event = ?event1, "Event 1");
        helper::assert_peer_joined(&event1, &net2.get_local_peer_id());

        let event2 = helper::get_next_peer_event(&mut events2).await;
        log::trace!(event = ?event2, "Event 2");
        helper::assert_peer_joined(&event2, &net1.get_local_peer_id());

        (net1, net2)
    }

    #[cfg(feature = "tokio-time")]
    async fn create_4_connected_networks() -> (
        (Network, Multiaddr),
        (Network, Multiaddr),
        (Network, Multiaddr),
        (Network, Multiaddr),
    ) {
        log::debug!("Creating connected test networks");
        let mut rng = thread_rng();
        let addr1 = multiaddr![Memory(rng.gen::<u64>())];
        let addr2 = multiaddr![Memory(rng.gen::<u64>())];
        let addr3 = multiaddr![Memory(rng.gen::<u64>())];
        let addr4 = multiaddr![Memory(rng.gen::<u64>())];

        let net1 = Network::new(
            network_config(addr1.clone()),
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        )
        .await;
        net1.listen_on(vec![addr1.clone()]).await;

        let net2 = Network::new(
            network_config(addr2.clone()),
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        )
        .await;
        net2.listen_on(vec![addr2.clone()]).await;

        let net3 = Network::new(
            network_config(addr3.clone()),
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        )
        .await;
        net3.listen_on(vec![addr3.clone()]).await;

        let net4 = Network::new(
            network_config(addr4.clone()),
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        )
        .await;
        net4.listen_on(vec![addr4.clone()]).await;

        log::debug!(address = %addr1, peer_id = %net1.get_local_peer_id(), "Network 1");
        log::debug!(address = %addr2, peer_id = %net2.get_local_peer_id(), "Network 2");
        log::debug!(address = %addr3, peer_id = %net3.get_local_peer_id(), "Network 3");
        log::debug!(address = %addr4, peer_id = %net4.get_local_peer_id(), "Network 4");

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();
        let mut events3 = net3.subscribe_events();
        let mut events4 = net4.subscribe_events();

        log::debug!("Dialing peer 1 from peer 2");
        net2.dial_address(addr1.clone()).await.unwrap();

        log::debug!("Dialing peer 1 from peer 3");
        net3.dial_address(addr1.clone()).await.unwrap();

        log::debug!("Dialing peer 1 from peer 4");
        net4.dial_address(addr1.clone()).await.unwrap();

        log::debug!("Waiting for join events");

        let event1 = helper::get_next_peer_event(&mut events1).await;
        log::trace!(event = ?event1, "Event 1");
        helper::assert_peer_joined(&event1, &net2.get_local_peer_id());

        let event2 = helper::get_next_peer_event(&mut events2).await;
        log::trace!(event = ?event2, "Event 2");
        helper::assert_peer_joined(&event2, &net1.get_local_peer_id());

        let event3 = helper::get_next_peer_event(&mut events3).await;
        log::trace!(event = ?event3, "Event 3");
        helper::assert_peer_joined(&event3, &net1.get_local_peer_id());

        let event4 = helper::get_next_peer_event(&mut events4).await;
        log::trace!(event = ?event4, "Event 4");
        helper::assert_peer_joined(&event4, &net1.get_local_peer_id());

        ((net1, addr1), (net2, addr2), (net3, addr3), (net4, addr4))
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

    let gossipsub = gossipsub::ConfigBuilder::default()
        .validation_mode(libp2p::gossipsub::ValidationMode::Permissive)
        .build()
        .expect("Invalid Gossipsub config");

    Config {
        keypair,
        peer_contact,
        seeds: Vec::new(),
        discovery: discovery::Config {
            genesis_hash: Default::default(),
            update_interval: Duration::from_secs(60),
            min_recv_update_interval: Duration::from_secs(30),
            update_limit: 64,
            required_services: Services::all(),
            min_send_update_interval: Duration::from_secs(30),
            house_keeping_interval: Duration::from_secs(60),
            keep_alive: true,
            only_secure_ws_connections: false,
        },
        kademlia: Default::default(),
        gossipsub,
        memory_transport: true,
        required_services: Services::all(),
        tls: None,
        desired_peer_count: 3,
        autonat_allow_non_global_ips: true,
        only_secure_ws_connections: false,
        allow_loopback_addresses: true,
    }
}

/// Listens to requests of type `ExpReq` and if `response` is `Some` then it
/// replies to the request using the `Req` type.
async fn respond_requests<Req: Request, ExpReq: Request + std::cmp::PartialEq>(
    network: Arc<Network>,
    response: Option<Req::Response>,
    expected_request: ExpReq,
) {
    // Subscribe for receiving requests
    let mut requests = network.receive_requests::<ExpReq>();
    log::info!("Waiting for Request message");
    let (received_request, request_id, peer_id) = requests.next().await.unwrap();
    log::info!(
        %request_id,
        %peer_id,
        request = ?received_request,
        "Received request"
    );
    assert_eq!(expected_request, received_request);

    // Respond the request if there is any
    if let Some(response) = response {
        assert!(network.respond::<Req>(request_id, response).await.is_ok());
    }
}

// Test that we can send a request and correctly receive the response given a proper
// request listener is replying in the peer specified
#[test(tokio::test)]
async fn test_valid_request_valid_response() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };
    let test_response = TestResponse { response: 43 };

    let net1 = Arc::new(net1);

    // Subscribe for receiving requests
    tokio::spawn({
        let net1 = Arc::clone(&net1);
        let test_request = test_request.clone();
        let test_response = test_response.clone();
        async move {
            respond_requests::<TestRequest, TestRequest>(net1, Some(test_response), test_request)
                .await
        }
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request");

    // Send the request and get future for the response
    let response = net2.request::<TestRequest>(test_request, net1.get_local_peer_id());

    // Check the received response
    let received_response = response.await;
    log::info!(response = ?received_response, "Received response");

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
    let (net1, net2) = TestNetwork::create_connected_networks().await;
    let peer_id_net1 = net1.get_local_peer_id();
    let net1 = Arc::new(net1);
    let mut response_futures = vec![];

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
                let result = net1
                    .respond::<TestRequest>(request_id, test_response.clone())
                    .await;
                assert!(result.is_ok());
            }
        });

    // Spawn the request listener future
    tokio::spawn(request_listener_future);

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending requests");

    for _ in 0..num_requests {
        // Send the request and get future for the response
        let response_future = net2.request::<TestRequest>(test_request.clone(), peer_id_net1);
        response_futures.push(response_future);
    }

    log::info!("Waiting for requests to arrive and check them");
    let responses = join_all(response_futures).await;
    for received_response in responses {
        match received_response {
            Ok(response) => {
                assert_eq!(response, test_response_2);
            }
            Err(e) => assert!(false, "Response received with error: {:?}", e),
        };
    }
}

// Test that we can send a request and receive a timeout response if no response is
// provided.
#[test(tokio::test)]
async fn test_valid_request_no_response() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };

    let net1 = Arc::new(net1);

    // Subscribe for receiving requests and don't let the function to respond to the request
    // (pass `None` to the response parameter)
    tokio::spawn({
        let net1 = Arc::clone(&net1);
        let test_request = test_request.clone();
        async move { respond_requests::<TestRequest, TestRequest>(net1, None, test_request).await }
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request");

    // Send the request and get future for the response
    let response = net2.request::<TestRequest>(test_request.clone(), net1.get_local_peer_id());

    // Since the request wasn't responded, it should timeout and send the error to the request
    let received_response = response.await;
    log::info!(response = ?received_response, "Received response");

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

// Test that we can send a request and receive a timeout response if no response is
// provided because the connection to a peer is closed in the middle of the request
#[test(tokio::test)]
async fn test_valid_request_no_response_close_connection() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;

    let test_request = TestRequest { request: 42 };

    let net1 = Arc::new(net1);
    let net2_peer_id = net2.get_local_peer_id();

    // Subscribe for receiving requests and don't let the function to respond to the request
    // (pass `None` to the response parameter)
    tokio::spawn({
        let net1 = Arc::clone(&net1);
        let test_request = test_request.clone();
        async move { respond_requests::<TestRequest, TestRequest>(net1, None, test_request).await }
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    log::info!("Sending request");

    // Send the request and get future for the response
    let response = net2.request::<TestRequest>(test_request.clone(), net1.get_local_peer_id());

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let net1 = Arc::clone(&net1);
        net1.disconnect_peer(net2_peer_id, CloseReason::MaliciousPeer)
            .await;
    });

    // Since the request wasn't responded, it should timeout and send the error to the request
    let received_response = response.await;
    log::info!(response = ?received_response, "Received response");

    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {:?}", response)
        }
        Err(e) => assert_eq!(
            e,
            RequestError::OutboundRequest(OutboundRequestError::ConnectionClosed)
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

    log::info!("Sending request");

    // Send the request and get future for the response
    let received_response = net2
        .request::<TestRequest>(test_request.clone(), net1.get_local_peer_id())
        .await;
    log::info!(response = ?received_response, "Received response");

    // Don't respond the request: It should timeout and send the error to the request
    // Check the received response
    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {:?}", response)
        }
        Err(e) => assert_eq!(
            e,
            RequestError::InboundRequest(InboundRequestError::NoReceiver)
        ),
    };
}

#[cfg(feature = "tokio-time")]
async fn disconnect_successfully(net1: &Arc<Network>, net2: &Arc<Network>) {
    log::debug!("Creating connected test networks");

    let mut events1 = net1.subscribe_events();
    let mut events2 = net2.subscribe_events();

    log::debug!("Disconnecting peer 1 from peer 2");
    net2.disconnect_peer(net1.get_local_peer_id(), CloseReason::GoingOffline)
        .await;

    log::debug!("Waiting for disconnect events");

    let event1 = helper::get_next_peer_event(&mut events1).await;
    log::trace!(event = ?event1, "Event 1");
    helper::assert_peer_left(&event1, &net2.get_local_peer_id());

    let event2 = helper::get_next_peer_event(&mut events2).await;
    log::trace!(event = ?event2, "Event 2");
    helper::assert_peer_left(&event2, &net1.get_local_peer_id());
}

#[cfg(feature = "tokio-time")]
async fn reconnect_successfully(net1: &Arc<Network>, addr1: Multiaddr, net2: &Arc<Network>) {
    log::debug!("Creating connected test networks");

    let mut events1 = net1.subscribe_events();
    let mut events2 = net2.subscribe_events();

    log::debug!("Dialing peer 1 from peer 2");
    net2.dial_address(addr1.clone()).await.unwrap();

    log::debug!("Waiting for join events");

    let event1 = helper::get_next_peer_event(&mut events1).await;
    log::trace!(event = ?event1, "Event 1");
    helper::assert_peer_joined(&event1, &net2.get_local_peer_id());

    let event2 = helper::get_next_peer_event(&mut events2).await;
    log::trace!(event = ?event2, "Event 2");
    helper::assert_peer_joined(&event2, &net1.get_local_peer_id());
}

#[cfg(feature = "tokio-time")]
async fn send_n_request_to_succeed(net1: &Arc<Network>, net2: &Arc<Network>, n: u32) {
    let test_request = TestRequest4 { request: 42 };
    let test_response = TestResponse4 { response: 43 };
    for i in 0..n {
        assert!(net2.has_peer(net1.get_local_peer_id()));
        assert!(net1.has_peer(net2.get_local_peer_id()));

        log::info!(
            "{:?} sends request {:?}",
            net1.get_local_peer_id(),
            test_request.clone(),
        );

        // Send the request and get future for the response.
        let response = net2.request::<TestRequest4>(test_request.clone(), net1.get_local_peer_id());

        log::info!("Succeed {:?}, {:?}", i, Instant::now());

        // Check the received response.
        let received_response = response.await;
        log::info!(response = ?received_response, "Received response");

        // Make sure that the request resolves as expected.
        match received_response {
            Ok(response) => {
                assert_eq!(
                    response, test_response,
                    "First requests must return Ok Result"
                );
            }
            Err(e) => assert!(false, "Response received with error: {:?}", e),
        };
    }
}

#[cfg(feature = "tokio-time")]
async fn send_n_request_to_fail(net1: &Arc<Network>, net2: &Arc<Network>, n: u32) {
    for i in 0..n {
        let test_request = TestRequest4 { request: 42 };
        let net1 = Arc::clone(&net1);
        let net2 = Arc::clone(&net2);
        assert!(net2.has_peer(net1.get_local_peer_id()));
        log::info!("Fail {:?}, {:?}", i, Instant::now());

        log::debug!(
            "{:?} sends request {:?}",
            net1.get_local_peer_id(),
            test_request.clone(),
        );
        let response = net2.request::<TestRequest4>(test_request.clone(), net1.get_local_peer_id());

        // Check the received response.
        let received_response = response.await;
        log::info!(response = ?received_response, "Received response");

        // Make sure the request timed out and did not return any other Error.
        assert_eq!(
            received_response,
            Err(RequestError::InboundRequest(
                InboundRequestError::ExceedsRateLimit
            )),
            "Subsequent requests must return Timeouts",
        );
    }
}

#[cfg(feature = "tokio-time")]
#[test(tokio::test)]
async fn it_can_limit_requests_rate() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;
    let net1 = Arc::new(net1);
    let net2 = Arc::new(net2);

    let test_response = TestResponse4 { response: 43 };

    // Subscribe for receiving requests.
    let request_stream = net1.receive_requests::<TestRequest4>();
    let network1 = Arc::clone(&net1);
    let request_listener_future =
        request_stream.for_each(move |(_request, request_id, _peer_id)| {
            let test_response = test_response.clone();
            let network1 = Arc::clone(&network1);
            async move {
                let _result = network1
                    .respond::<TestRequest4>(request_id, test_response.clone())
                    .await;
            }
        });

    // Spawn the request listener future.
    tokio::spawn(request_listener_future);

    tokio::time::sleep(Duration::from_secs(1)).await;

    tokio::time::pause();
    log::error!("Clock stops at {:?}", Instant::now());

    // The first head request is sent and this should be the only request that gets an Ok response.
    // This sets the last reset of the rate limiting to block height 1 (current block height).
    send_n_request_to_succeed(&net1, &net2, TestRequest4::MAX_REQUESTS).await;

    tokio::time::advance(TestRequest4::TIME_WINDOW - Duration::from_secs(1)).await;
    // Elapsed time 99 secs;

    log::error!("Advanced time to {:?}", Instant::now());

    // Rate limit was exceeded, and reset should only happen after 10 blocks from the first the first request sent.
    send_n_request_to_fail(&net1, &net2, 5).await;

    // Block height is is now 11, so a reset for the rate limiting should happen.
    tokio::time::advance(Duration::from_secs(1)).await;
    // Elapsed time 100 secs;

    send_n_request_to_succeed(&net1, &net2, TestRequest4::MAX_REQUESTS).await;

    // Rate limit was exceeded, from now on it should fail with a timeout.
    send_n_request_to_fail(&net1, &net2, 5).await;

    // Advance time to reset the counters once more.
    tokio::time::advance(TestRequest4::TIME_WINDOW).await;
    // Elapsed time 200 secs;

    // Counters should be reset, new requests are allowed.
    send_n_request_to_succeed(&net1, &net2, TestRequest4::MAX_REQUESTS).await;
}

#[cfg(feature = "tokio-time")]
#[test(tokio::test)]
async fn it_can_limit_requests_rate_after_reconnection() {
    let ((net1, addr1), (net2, _), (net3, _), (net4, _)) =
        TestNetwork::create_4_connected_networks().await;
    let net1 = Arc::new(net1);
    let net2 = Arc::new(net2);
    let net3 = Arc::new(net3);
    let net4 = Arc::new(net4);

    let test_response = TestResponse4 { response: 43 };

    // Subscribe for receiving requests.
    let request_stream = net1.receive_requests::<TestRequest4>();
    let network1 = Arc::clone(&net1);
    let request_listener_future =
        request_stream.for_each(move |(_request, request_id, _peer_id)| {
            let test_response = test_response.clone();
            let network1 = Arc::clone(&network1);
            async move {
                let _result = network1
                    .respond::<TestRequest4>(request_id, test_response.clone())
                    .await;
            }
        });

    // Spawn the request listener future.
    tokio::spawn(request_listener_future);

    tokio::time::sleep(Duration::from_secs(1)).await;

    // The first head requests are sent. These should be the only requests that get an Ok response.
    send_n_request_to_succeed(&net1, &net2, TestRequest4::MAX_REQUESTS).await;

    // Disconnects peer2.
    disconnect_successfully(&net1, &net2).await;

    // Make the first and second request from peer3.
    send_n_request_to_succeed(&net1, &net3, TestRequest4::MAX_REQUESTS).await;

    // Disconnects peer3.
    disconnect_successfully(&net1, &net3).await;

    // Reconnect peer2.
    reconnect_successfully(&net1, addr1.clone(), &net2).await;

    // Rate limit was exceeded for peer 2, from now on it should fail with a timeout.
    send_n_request_to_fail(&net1, &net2, 1).await;

    // Disconnects peer2.
    disconnect_successfully(&net1, &net2).await;

    // Reconnect peer3.
    reconnect_successfully(&net1, addr1, &net3).await;

    // Rate limit was exceeded, from now on it should fail with a timeout.
    send_n_request_to_fail(&net1, &net3, 1).await;

    // Make the first and second request from peer4.
    send_n_request_to_succeed(&net1, &net4, TestRequest4::MAX_REQUESTS).await;
}

#[cfg(feature = "tokio-time")]
#[test(tokio::test)]
async fn it_can_reset_requests_rate_with_reconnections() {
    let ((net1, addr1), (net2, _), (net3, _), _) = TestNetwork::create_4_connected_networks().await;
    let net1 = Arc::new(net1);
    let net2 = Arc::new(net2);
    let net3 = Arc::new(net3);

    let test_response = TestResponse4 { response: 43 };

    // Subscribe for receiving requests.
    let request_stream = net1.receive_requests::<TestRequest4>();
    let network1 = Arc::clone(&net1);
    let request_listener_future =
        request_stream.for_each(move |(_request, request_id, _peer_id)| {
            let test_response = test_response.clone();
            let network1 = Arc::clone(&network1);
            async move {
                let _result = network1
                    .respond::<TestRequest4>(request_id, test_response.clone())
                    .await;
            }
        });

    // Spawn the request listener future.
    tokio::spawn(request_listener_future);

    tokio::time::sleep(Duration::from_secs(1)).await;

    tokio::time::pause();

    // The first head requests are sent. These should be the only requests that get an Ok response.
    send_n_request_to_succeed(&net1, &net2, TestRequest4::MAX_REQUESTS).await;
    // Disconnects peer 2.
    disconnect_successfully(&net1, &net2).await;

    tokio::time::advance(TestRequest4::TIME_WINDOW / 2).await;
    // time passed 50 secs;

    // Make the first and second request from peer 3.
    // The expiration block of these rates should be the current time elapsed + time window (50+100=150 secs).
    send_n_request_to_succeed(&net1, &net3, TestRequest4::MAX_REQUESTS).await;
    // Disconnects peer 3.
    disconnect_successfully(&net1, &net3).await;

    // Puts the total elapsed time to 149 secs, the counters for peer 2 are to be reset (expiration is 100).
    tokio::time::advance(TestRequest4::TIME_WINDOW - Duration::from_secs(1)).await;
    // time passed 149 secs;

    // Reconnect peer 3 and ensure the requests fail, no reset should have happened yet.
    reconnect_successfully(&net1, addr1.clone(), &net3).await;
    send_n_request_to_fail(&net1, &net3, 1).await;

    // Reconnect peer 2 and ensure the requests succeed, the reset should happen.
    reconnect_successfully(&net1, addr1, &net2).await;
    send_n_request_to_succeed(&net1, &net2, TestRequest4::MAX_REQUESTS).await;

    // Puts the total elapsed time to 150 secs, only resets counters for peer 3.
    tokio::time::advance(Duration::from_secs(1)).await;
    // time passed 150 secs;

    send_n_request_to_succeed(&net1, &net3, TestRequest4::MAX_REQUESTS).await;

    send_n_request_to_fail(&net1, &net3, 1).await;
    send_n_request_to_fail(&net1, &net2, 1).await;
}
