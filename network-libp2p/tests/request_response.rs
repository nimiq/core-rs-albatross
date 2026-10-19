use std::{num::NonZeroU8, sync::Arc, time::Duration};

use futures::{future::join_all, StreamExt};
use instant::SystemTime;
use libp2p::{
    core::multiaddr::{multiaddr, Multiaddr},
    gossipsub,
    identity::Keypair,
};
use nimiq_network_interface::{
    network::{CloseReason, Network as NetworkInterface},
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
use nimiq_time::sleep;
use nimiq_utils::spawn;

mod helper;

/// The max number of TestRequests per peer (used for regular tests only).
const MAX_REQUEST_RESPONSE_TEST_REQUEST: u32 = 1000;
/// The budget of `TestRequest4`, used by the reconnection test: two requests per burst.
const RECONNECTION_TEST_BURST: u32 = 2;
/// The refill window of `TestRequest4`, long enough that no request comes back during the
/// reconnection test.
const RECONNECTION_TEST_WINDOW: Duration = Duration::from_secs(100);

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
struct TestRequest4 {
    request: u64,
}
impl RequestCommon for TestRequest4 {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 44;
    type Response = TestResponse4;

    const MAX_REQUESTS: u32 = RECONNECTION_TEST_BURST;
    const TIME_WINDOW: Duration = RECONNECTION_TEST_WINDOW;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestResponse4 {
    response: u32,
}

/// The budget of `TestRequest5`: two requests per burst, one back every `RATE_LIMIT_TEST_REFILL`.
const RATE_LIMIT_TEST_BURST: u32 = 2;
/// The time it takes to refill the whole budget of `TestRequest5`.
const RATE_LIMIT_TEST_WINDOW: Duration = Duration::from_secs(6);
/// How long one request takes to come back.
const RATE_LIMIT_TEST_REFILL: Duration =
    Duration::from_secs(RATE_LIMIT_TEST_WINDOW.as_secs() / RATE_LIMIT_TEST_BURST as u64);
/// Margin added to every wait, so a slow test runner does not land on a refill boundary.
const RATE_LIMIT_TEST_MARGIN: Duration = Duration::from_millis(500);

/// Request type with a short rate limit window, used by the real-time rate limit tests.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TestRequest5 {
    request: u64,
}
impl RequestCommon for TestRequest5 {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 50;
    type Response = TestResponse4;

    const MAX_REQUESTS: u32 = RATE_LIMIT_TEST_BURST;
    const TIME_WINDOW: Duration = RATE_LIMIT_TEST_WINDOW;
}

#[derive(Clone, Debug)]
struct TestNetwork {}

impl TestNetwork {
    async fn create_connected_networks() -> (Network, Network) {
        log::debug!("Creating connected test networks");
        let addr1 = multiaddr![Memory(rand::random::<u64>())];
        let addr2 = multiaddr![Memory(rand::random::<u64>())];

        let net1 = Network::new(network_config(addr1.clone()), ()).await;
        net1.listen_on(vec![addr1.clone()]).await.unwrap();

        let net2 = Network::new(network_config(addr2.clone()), ()).await;
        net2.listen_on(vec![addr2.clone()]).await.unwrap();

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

    async fn create_4_connected_networks() -> (
        (Network, Multiaddr),
        (Network, Multiaddr),
        (Network, Multiaddr),
        (Network, Multiaddr),
    ) {
        log::debug!("Creating connected test networks");
        let addr1 = multiaddr![Memory(rand::random::<u64>())];
        let addr2 = multiaddr![Memory(rand::random::<u64>())];
        let addr3 = multiaddr![Memory(rand::random::<u64>())];
        let addr4 = multiaddr![Memory(rand::random::<u64>())];

        let net1 = Network::new(network_config(addr1.clone()), ()).await;
        net1.listen_on(vec![addr1.clone()]).await.unwrap();

        let net2 = Network::new(network_config(addr2.clone()), ()).await;
        net2.listen_on(vec![addr2.clone()]).await.unwrap();

        let net3 = Network::new(network_config(addr3.clone()), ()).await;
        net3.listen_on(vec![addr3.clone()]).await.unwrap();

        let net4 = Network::new(network_config(addr4.clone()), ()).await;
        net4.listen_on(vec![addr4.clone()]).await.unwrap();

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

    let peer_contact = PeerContact::new(
        Some(address),
        keypair.public(),
        Services::all(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .expect("PeerContact must be creatable");

    let gossipsub = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Permissive)
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
        num_initial_connections: 1,
        peer_count_max: 4000,
        peer_count_per_ip_max: 20,
        peer_count_per_subnet_max: 20,
        only_secure_ws_connections: false,
        allow_loopback_addresses: true,
        dht_quorum: NonZeroU8::new(1).unwrap(),
        network_buffer_size: 1024,
    }
}

/// Listens to requests of type `ExpReq` and if `response` is `Some` then it
/// replies to the request using the `Req` type.
async fn respond_requests<Req: Request, ExpReq: Request + PartialEq>(
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
    spawn({
        let net1 = Arc::clone(&net1);
        let test_request = test_request.clone();
        let test_response = test_response.clone();
        async move {
            respond_requests::<TestRequest, TestRequest>(net1, Some(test_response), test_request)
                .await
        }
    });

    sleep(Duration::from_secs(1)).await;

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
        Err(e) => assert!(false, "Response received with error: {e:?}"),
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
    spawn(request_listener_future);

    sleep(Duration::from_secs(1)).await;

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
            Err(e) => assert!(false, "Response received with error: {e:?}"),
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
    spawn({
        let net1 = Arc::clone(&net1);
        let test_request = test_request.clone();
        async move { respond_requests::<TestRequest, TestRequest>(net1, None, test_request).await }
    });

    sleep(Duration::from_secs(1)).await;

    log::info!("Sending request");

    // Send the request and get future for the response
    let response = net2.request::<TestRequest>(test_request.clone(), net1.get_local_peer_id());

    // Since the request wasn't responded, it should timeout and send the error to the request
    let received_response = response.await;
    log::info!(response = ?received_response, "Received response");

    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {response:?}")
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
    spawn({
        let net1 = Arc::clone(&net1);
        let test_request = test_request.clone();
        async move { respond_requests::<TestRequest, TestRequest>(net1, None, test_request).await }
    });

    sleep(Duration::from_secs(1)).await;

    log::info!("Sending request");

    // Send the request and get future for the response
    let response = net2.request::<TestRequest>(test_request.clone(), net1.get_local_peer_id());

    spawn(async move {
        sleep(Duration::from_secs(1)).await;
        let net1 = Arc::clone(&net1);
        net1.disconnect_peer(net2_peer_id, CloseReason::MaliciousPeer)
            .await;
    });

    // Since the request wasn't responded, it should timeout and send the error to the request
    let received_response = response.await;
    log::info!(response = ?received_response, "Received response");

    match received_response {
        Ok(response) => {
            assert!(false, "Received unexpected valid response: {response:?}")
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

    sleep(Duration::from_secs(1)).await;

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
            assert!(false, "Received unexpected valid response: {response:?}")
        }
        Err(e) => assert_eq!(
            e,
            RequestError::InboundRequest(InboundRequestError::NoReceiver)
        ),
    };
}

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

/// Answers every `R` request that `server` receives with `TestResponse4 { response: 43 }`.
fn spawn_responder<R>(server: &Arc<Network>)
where
    R: Request<Response = TestResponse4>,
{
    let request_stream = server.receive_requests::<R>();
    let server = Arc::clone(server);
    spawn(
        request_stream.for_each(move |(_request, request_id, _peer_id)| {
            let server = Arc::clone(&server);
            async move {
                let _result = server
                    .respond::<R>(request_id, TestResponse4 { response: 43 })
                    .await;
            }
        }),
    );
}

/// Sends `request` `n` times from `client` to `server` and expects every one to be answered.
async fn send_n_request_to_succeed<R>(
    server: &Arc<Network>,
    client: &Arc<Network>,
    request: R,
    n: u32,
) where
    R: Request<Response = TestResponse4> + Clone,
{
    for i in 0..n {
        assert!(client.has_peer(server.get_local_peer_id()));
        assert!(server.has_peer(client.get_local_peer_id()));
        log::info!(request = ?request, i, "Sending a request that must succeed");
        let response = client
            .request::<R>(request.clone(), server.get_local_peer_id())
            .await;
        log::info!(?response, "Received response");
        assert_eq!(
            response,
            Ok(TestResponse4 { response: 43 }),
            "request {i} must be answered",
        );
    }
}

/// Sends `request` `n` times from `client` to `server` and expects every one to be refused
/// because the client exceeded its rate limit.
async fn send_n_request_to_fail<R>(server: &Arc<Network>, client: &Arc<Network>, request: R, n: u32)
where
    R: Request<Response = TestResponse4> + Clone,
{
    for i in 0..n {
        assert!(client.has_peer(server.get_local_peer_id()));
        assert!(server.has_peer(client.get_local_peer_id()));
        log::info!(request = ?request, i, "Sending a request that must be rate limited");
        let response = client
            .request::<R>(request.clone(), server.get_local_peer_id())
            .await;
        log::info!(?response, "Received response");
        assert_eq!(
            response,
            Err(RequestError::InboundRequest(
                InboundRequestError::ExceedsRateLimit
            )),
            "request {i} must be rate limited",
        );
    }
}

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
    spawn(request_listener_future);

    sleep(Duration::from_secs(1)).await;

    let request = TestRequest4 { request: 42 };

    // The first head requests are sent. These should be the only requests that get an Ok response.
    send_n_request_to_succeed(&net1, &net2, request.clone(), TestRequest4::MAX_REQUESTS).await;

    // Disconnects peer2.
    disconnect_successfully(&net1, &net2).await;

    // Make the first and second request from peer3.
    send_n_request_to_succeed(&net1, &net3, request.clone(), TestRequest4::MAX_REQUESTS).await;

    // Disconnects peer3.
    disconnect_successfully(&net1, &net3).await;

    // Reconnect peer2.
    reconnect_successfully(&net1, addr1.clone(), &net2).await;

    // Peer 2 spent its budget before leaving and nothing has refilled, so it is still rate limited.
    send_n_request_to_fail(&net1, &net2, request.clone(), 1).await;

    // Disconnects peer2.
    disconnect_successfully(&net1, &net2).await;

    // Reconnect peer3.
    reconnect_successfully(&net1, addr1, &net3).await;

    // Same for peer 3.
    send_n_request_to_fail(&net1, &net3, request.clone(), 1).await;

    // Make the first and second request from peer4.
    send_n_request_to_succeed(&net1, &net4, request, TestRequest4::MAX_REQUESTS).await;
}

/// Requests beyond the burst are refused and come back one per refill interval.
#[test(tokio::test)]
async fn it_can_limit_requests_rate() {
    let (net1, net2) = TestNetwork::create_connected_networks().await;
    let net1 = Arc::new(net1);
    let net2 = Arc::new(net2);
    spawn_responder::<TestRequest5>(&net1);
    sleep(Duration::from_secs(1)).await;

    let request = TestRequest5 { request: 42 };

    // The whole burst is served, the next requests find the bucket empty.
    send_n_request_to_succeed(&net1, &net2, request.clone(), RATE_LIMIT_TEST_BURST).await;
    send_n_request_to_fail(&net1, &net2, request.clone(), 2).await;

    // One refill interval later exactly one request is served again.
    sleep(RATE_LIMIT_TEST_REFILL + RATE_LIMIT_TEST_MARGIN).await;
    send_n_request_to_succeed(&net1, &net2, request.clone(), 1).await;
    send_n_request_to_fail(&net1, &net2, request.clone(), 1).await;

    // A whole window later the bucket is full again.
    sleep(RATE_LIMIT_TEST_WINDOW + RATE_LIMIT_TEST_MARGIN).await;
    send_n_request_to_succeed(&net1, &net2, request.clone(), RATE_LIMIT_TEST_BURST).await;
    send_n_request_to_fail(&net1, &net2, request, 1).await;
}

/// A peer's budget survives a reconnect and keeps refilling while the peer is away.
#[test(tokio::test)]
async fn it_keeps_refilling_while_a_peer_is_away() {
    let ((net1, addr1), (net2, _), (net3, _), _) = TestNetwork::create_4_connected_networks().await;
    let net1 = Arc::new(net1);
    let net2 = Arc::new(net2);
    let net3 = Arc::new(net3);
    spawn_responder::<TestRequest5>(&net1);
    sleep(Duration::from_secs(1)).await;

    let request = TestRequest5 { request: 42 };

    // Peer 2 spends its whole budget and peer 3 half of it, then both leave.
    send_n_request_to_succeed(&net1, &net2, request.clone(), RATE_LIMIT_TEST_BURST).await;
    send_n_request_to_succeed(&net1, &net3, request.clone(), 1).await;
    disconnect_successfully(&net1, &net2).await;
    disconnect_successfully(&net1, &net3).await;

    // One refill interval later peer 2 has one request back and peer 3 is full again.
    sleep(RATE_LIMIT_TEST_REFILL + RATE_LIMIT_TEST_MARGIN).await;
    reconnect_successfully(&net1, addr1.clone(), &net2).await;
    reconnect_successfully(&net1, addr1.clone(), &net3).await;
    send_n_request_to_succeed(&net1, &net2, request.clone(), 1).await;
    send_n_request_to_fail(&net1, &net2, request.clone(), 1).await;
    send_n_request_to_succeed(&net1, &net3, request.clone(), RATE_LIMIT_TEST_BURST).await;
    send_n_request_to_fail(&net1, &net3, request.clone(), 1).await;

    // Leaving and coming back right away does not reset anything.
    disconnect_successfully(&net1, &net3).await;
    reconnect_successfully(&net1, addr1, &net3).await;
    send_n_request_to_fail(&net1, &net3, request, 1).await;
}
