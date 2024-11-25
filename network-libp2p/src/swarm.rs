use std::{collections::HashMap, num::NonZeroU8, sync::Arc};

use futures::StreamExt;
#[cfg(feature = "metrics")]
use instant::Instant;
#[cfg(all(target_family = "wasm", not(feature = "tokio-websocket")))]
use libp2p::websocket_websys;
use libp2p::{
    autonat::{self, InboundFailure, OutboundFailure},
    core::{
        self,
        muxing::StreamMuxerBox,
        transport::{Boxed, MemoryTransport},
    },
    gossipsub,
    identity::Keypair,
    kad::{
        self, store::RecordStore, BootstrapError, BootstrapOk, GetRecordError, GetRecordOk,
        InboundRequest, Mode, ProgressStep, PutRecordError, PutRecordOk, QueryId, QueryResult,
        QueryStats, Quorum, Record,
    },
    noise, ping,
    request_response::{self, InboundRequestId, OutboundRequestId, ResponseChannel},
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        ConnectionId, SwarmEvent,
    },
    yamux, PeerId, Swarm, SwarmBuilder, Transport,
};
#[cfg(feature = "tokio-websocket")]
use libp2p::{dns, tcp, websocket};
use log::Instrument;
use nimiq_network_interface::{
    network::{CloseReason, NetworkEvent},
    peer_info::PeerInfo,
    request::{peek_type, InboundRequestError, OutboundRequestError, RequestError},
};
use nimiq_serde::Serialize;
use nimiq_time::Interval;
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::{
    autonat::NatStatus,
    behaviour, dht,
    discovery::{self, peer_contacts::PeerContactBook},
    network_types::{
        DhtBootStrapState, DhtRecord, DhtResults, GossipsubTopicInfo, NetworkAction, TaskState,
        ValidateMessage,
    },
    rate_limiting::{RateLimitId, RateLimits},
    Config, NetworkError, TlsConfig,
};

type NimiqSwarm = Swarm<behaviour::Behaviour>;

struct EventInfo<'a> {
    events_tx: &'a broadcast::Sender<NetworkEvent<PeerId>>,
    swarm: &'a mut NimiqSwarm,
    state: &'a mut TaskState,
    connected_peers: &'a RwLock<HashMap<PeerId, PeerInfo>>,
    rate_limiting: &'a mut RateLimits,
    #[cfg(feature = "kad")]
    dht_verifier: &'a dyn dht::Verifier,
    #[cfg(feature = "metrics")]
    metrics: &'a Arc<NetworkMetrics>,
}

pub(crate) fn new_swarm(
    config: Config,
    contacts: Arc<RwLock<PeerContactBook>>,
    peer_score_params: gossipsub::PeerScoreParams,
    force_dht_server_mode: bool,
) -> Swarm<behaviour::Behaviour> {
    let keypair = config.keypair.clone();
    let transport = new_transport(
        &keypair,
        config.memory_transport,
        config.only_secure_ws_connections,
        config.tls.as_ref(),
    )
    .unwrap();

    let behaviour =
        behaviour::Behaviour::new(config, contacts, peer_score_params, force_dht_server_mode);

    // TODO add proper config
    #[cfg(not(target_family = "wasm"))]
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_other_transport(|_| transport)
        .unwrap()
        .with_behaviour(|_| behaviour)
        .unwrap()
        .build();
    #[cfg(target_family = "wasm")]
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_wasm_bindgen()
        .with_other_transport(|_| transport)
        .unwrap()
        .with_behaviour(|_| behaviour)
        .unwrap()
        .build();
    swarm
}

pub(crate) async fn swarm_task(
    mut swarm: NimiqSwarm,
    events_tx: broadcast::Sender<NetworkEvent<PeerId>>,
    mut action_rx: mpsc::Receiver<NetworkAction>,
    mut validate_rx: mpsc::UnboundedReceiver<ValidateMessage<PeerId>>,
    connected_peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    mut update_scores: Interval,
    contacts: Arc<RwLock<PeerContactBook>>,
    #[cfg(feature = "kad")] dht_verifier: impl dht::Verifier,
    force_dht_server_mode: bool,
    dht_quorum: NonZeroU8,
    #[cfg(feature = "metrics")] metrics: Arc<NetworkMetrics>,
) {
    let mut task_state = TaskState {
        dht_server_mode: force_dht_server_mode,
        dht_quorum: dht_quorum.into(),
        ..Default::default()
    };
    let mut rate_limiting = RateLimits::default();

    let peer_id = Swarm::local_peer_id(&swarm);
    let task_span = trace_span!("swarm task", peer_id=?peer_id);

    async move {
        loop {
            tokio::select! {
                validate_msg = validate_rx.recv() => {
                    if let Some(validate_msg) = validate_msg {
                        let topic = validate_msg.topic;
                        let result: Result<bool, gossipsub::PublishError> = swarm
                            .behaviour_mut()
                            .gossipsub
                            .report_message_validation_result(
                                &validate_msg.pubsub_id.message_id,
                                &validate_msg.pubsub_id.propagation_source,
                                validate_msg.acceptance,
                            );

                        match result {
                            Ok(true) => {}, // success
                            Ok(false) => debug!(topic, "Validation took too long: message is no longer in the message cache"),
                            Err(e) => error!(topic, error = %e, "Network error while relaying message"),
                        }
                    }
                },
                event = swarm.next() => {
                    if let Some(event) = event {
                        handle_event(
                            event,
                            EventInfo::<'_> {
                                events_tx: &events_tx,
                                swarm: &mut swarm,
                                state: &mut task_state,
                                connected_peers: &connected_peers,
                                rate_limiting: &mut rate_limiting,
                                #[cfg(feature = "kad")]
                                dht_verifier: &dht_verifier,
                                #[cfg( feature = "metrics")] metrics: &metrics,
                            },
                        );
                    }
                },
                action = action_rx.recv() => {
                    if let Some(action) = action {
                        perform_action(action, &mut swarm, &mut task_state);
                    }
                    else {
                        // `action_rx.next()` will return `None` if all senders (i.e. the `Network` object) are dropped.
                        break;
                    }
                },
                _ = update_scores.next() => {
                    swarm.behaviour().update_scores(Arc::clone(&contacts));
                },
            };
        }
    }
    .instrument(task_span)
    .await
}

fn new_transport(
    keypair: &Keypair,
    memory_transport: bool,
    only_secure_ws_connections: bool,
    tls: Option<&TlsConfig>,
) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
    let yamux = yamux::Config::default();

    if memory_transport {
        // Memory transport primary for testing
        // TODO: Use websocket over the memory transport

        #[cfg(feature = "tokio-websocket")]
        let mut transport = websocket::WsConfig::new(dns::tokio::Transport::system(
            tcp::tokio::Transport::new(tcp::Config::default().nodelay(true)),
        )?);

        // Configure TLS if the configuration has the corresponding entry
        #[cfg(feature = "tokio-websocket")]
        if let Some(tls) = tls {
            let priv_key = websocket::tls::PrivateKey::new(tls.private_key.clone());
            let certificates: Vec<_> = tls
                .certificates
                .clone()
                .into_iter()
                .map(websocket::tls::Certificate::new)
                .collect();
            transport.set_tls_config(websocket::tls::Config::new(priv_key, certificates).unwrap());
        }

        #[cfg(not(feature = "tokio-websocket"))]
        let _ = tls; // silence unused variable warning

        #[cfg(feature = "tokio-websocket")]
        let transport = transport.or_transport(MemoryTransport::default());

        #[cfg(not(feature = "tokio-websocket"))]
        let transport = MemoryTransport::default();

        if only_secure_ws_connections {
            Ok(crate::only_secure_ws_transport::Transport::new(transport)
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::Config::new(keypair).unwrap())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        } else {
            Ok(transport
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::Config::new(keypair).unwrap())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        }
    } else {
        #[cfg(feature = "tokio-websocket")]
        let mut transport = websocket::WsConfig::new(dns::tokio::Transport::system(
            tcp::tokio::Transport::new(tcp::Config::default().nodelay(true)),
        )?);

        // Configure TLS if the configuration has the corresponding entry
        #[cfg(feature = "tokio-websocket")]
        if let Some(tls) = tls {
            let priv_key = websocket::tls::PrivateKey::new(tls.private_key.clone());
            let certificates: Vec<_> = tls
                .certificates
                .clone()
                .into_iter()
                .map(websocket::tls::Certificate::new)
                .collect();
            transport.set_tls_config(websocket::tls::Config::new(priv_key, certificates).unwrap());
        }

        #[cfg(all(target_family = "wasm", not(feature = "tokio-websocket")))]
        let transport = websocket_websys::Transport::default();

        #[cfg(all(not(feature = "tokio-websocket"), not(target_family = "wasm")))]
        let transport = MemoryTransport::default();

        if only_secure_ws_connections {
            Ok(crate::only_secure_ws_transport::Transport::new(transport)
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::Config::new(keypair).unwrap())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        } else {
            Ok(transport
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::Config::new(keypair).unwrap())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        }
    }
}

fn handle_event(event: SwarmEvent<behaviour::BehaviourEvent>, event_info: EventInfo) {
    match event {
        SwarmEvent::ConnectionEstablished {
            connection_id,
            peer_id,
            endpoint,
            num_established,
            concurrent_dial_errors,
            established_in,
        } => {
            debug!(
                %connection_id,
                %peer_id,
                address = %endpoint.get_remote_address(),
                direction = if endpoint.is_dialer() { "outbound" } else { "inbound" },
                connections = num_established,
                ?established_in,
                "Connection established",
            );

            if let Some(dial_errors) = concurrent_dial_errors {
                for (addr, error) in dial_errors {
                    trace!(%peer_id, address = %addr, %error, "Removing addresses that caused dial failures");
                    event_info
                        .swarm
                        .behaviour_mut()
                        .remove_peer_address(peer_id, addr);
                }
            }

            // Save dialed peer addresses
            if endpoint.is_dialer() {
                let listen_addr = endpoint.get_remote_address();

                if event_info
                    .swarm
                    .behaviour()
                    .is_address_dialable(listen_addr)
                {
                    debug!(%peer_id, address = %listen_addr, "Saving peer");

                    event_info
                        .swarm
                        .behaviour_mut()
                        .add_peer_address(peer_id, listen_addr.clone());

                    // Bootstrap Kademlia if we're performing our first connection
                    #[cfg(feature = "kad")]
                    if event_info.state.dht_bootstrap_state == DhtBootStrapState::NotStarted {
                        debug!("Bootstrapping DHT");
                        if event_info.swarm.behaviour_mut().dht.bootstrap().is_err() {
                            error!("Bootstrapping DHT error: No known peers");
                        }
                        event_info.state.dht_bootstrap_state = DhtBootStrapState::Started;
                    }
                }
            }
        }

        SwarmEvent::ConnectionClosed {
            connection_id,
            peer_id,
            endpoint,
            num_established,
            cause,
        } => {
            info!(%connection_id, %peer_id, ?endpoint, connections = num_established, "Connection closed with peer");

            if let Some(cause) = cause {
                info!(%cause, "Connection closed because");
            }

            // Remove Peer
            if num_established == 0 {
                event_info.connected_peers.write().remove(&peer_id);
                event_info.swarm.behaviour_mut().remove_peer(peer_id);

                // Removes or marks to remove the respective rate limits.
                // Also cleans up the expired rate limits pending to delete.
                event_info.rate_limiting.remove_rate_limits(peer_id);

                let _ = event_info.events_tx.send(NetworkEvent::PeerLeft(peer_id));
            }
        }
        SwarmEvent::IncomingConnection {
            connection_id,
            local_addr,
            send_back_addr,
        } => {
            debug!(%connection_id, address = %send_back_addr, listen_address = %local_addr, "Incoming connection");
        }

        SwarmEvent::IncomingConnectionError {
            connection_id,
            local_addr,
            send_back_addr,
            error,
        } => {
            debug!(%connection_id, address = %send_back_addr, listen_address = %local_addr, %error, "Incoming connection error");
        }

        SwarmEvent::Dialing {
            peer_id: Some(peer_id),
            connection_id: _,
        } => {
            // This event is only triggered if the network behaviour performs the dial
            debug!(%peer_id, "Dialing peer");
        }

        SwarmEvent::NewListenAddr {
            listener_id: _,
            address,
        } => {
            debug!(%address, "New listen address");
            event_info
                .swarm
                .behaviour_mut()
                .discovery
                .add_own_addresses([address.clone()].to_vec());
            if event_info.swarm.behaviour().is_address_dialable(&address) {
                event_info.state.nat_status.add_address(address);
            }
        }

        SwarmEvent::ListenerClosed {
            listener_id: _,
            addresses,
            reason: _,
        } => {
            addresses.iter().for_each(|address| {
                event_info.state.nat_status.remove_address(address);
            });
        }

        SwarmEvent::ExternalAddrConfirmed { address } => {
            log::trace!(%address, "Address is confirmed and externally reachable");
            event_info.state.nat_status.add_confirmed_address(address);
        }

        SwarmEvent::ExternalAddrExpired { address } => {
            log::trace!(%address, "External address is expired and no longer externally reachable");
            event_info
                .state
                .nat_status
                .remove_confirmed_address(&address);
        }

        SwarmEvent::Behaviour(event) => handle_behaviour_event(event, event_info),

        _ => {}
    }
}

fn handle_behaviour_event(event: behaviour::BehaviourEvent, event_info: EventInfo) {
    match event {
        behaviour::BehaviourEvent::AutonatClient(event) => {
            handle_autonat_client_event(event, event_info)
        }
        behaviour::BehaviourEvent::AutonatServer(event) => {
            handle_autonat_server_event(event, event_info)
        }
        behaviour::BehaviourEvent::ConnectionLimits(event) => match event {},
        behaviour::BehaviourEvent::Pool(event) => match event {},
        #[cfg(feature = "kad")]
        behaviour::BehaviourEvent::Dht(event) => handle_dht_event(event, event_info),
        behaviour::BehaviourEvent::Discovery(event) => handle_discovery_event(event, event_info),
        behaviour::BehaviourEvent::Gossipsub(event) => handle_gossipsup_event(event, event_info),
        behaviour::BehaviourEvent::Ping(event) => handle_ping_event(event, event_info),
        behaviour::BehaviourEvent::RequestResponse(event) => {
            handle_request_response_event(event, event_info)
        }
    }
}

fn handle_autonat_client_event(event: autonat::v2::client::Event, event_info: EventInfo) {
    log::trace!(?event, "AutoNAT outbound probe");
    match event.result {
        Ok(_) => event_info
            .state
            .nat_status
            .set_address_nat(event.tested_addr, NatStatus::Public),
        Err(_) => event_info
            .state
            .nat_status
            .set_address_nat(event.tested_addr, NatStatus::Private),
    }
}

fn handle_autonat_server_event(event: autonat::v2::server::Event, _event_info: EventInfo) {
    log::trace!(?event, "AutoNAT inbound probe");
}

#[cfg(feature = "kad")]
fn handle_dht_event(event: kad::Event, event_info: EventInfo) {
    match event {
        kad::Event::OutboundQueryProgressed {
            id,
            result: QueryResult::GetRecord(result),
            stats,
            step,
        } => handle_dht_get(id, result, stats, step, event_info),

        kad::Event::OutboundQueryProgressed {
            id,
            result: QueryResult::PutRecord(result),
            stats,
            step,
        } => handle_dht_put_record(id, result, stats, step, event_info),

        kad::Event::OutboundQueryProgressed {
            id,
            result: QueryResult::Bootstrap(result),
            stats,
            step,
        } => handle_dht_bootstrap(id, result, stats, step, event_info),

        kad::Event::InboundRequest {
            request:
                InboundRequest::PutRecord {
                    source,
                    connection,
                    record: Some(record),
                },
        } => handle_dht_inbound_put(source, connection, record, event_info),

        kad::Event::ModeChanged { new_mode } => handle_dht_mode_change(new_mode, event_info),

        _ => {}
    }
}

#[cfg(feature = "kad")]
fn handle_dht_get(
    id: QueryId,
    result: Result<GetRecordOk, GetRecordError>,
    _stats: QueryStats,
    step: ProgressStep,
    event_info: EventInfo,
) {
    match result {
        Ok(GetRecordOk::FoundRecord(record)) => {
            // Verify incoming record
            let dht_record = match event_info.dht_verifier.verify(&record.record) {
                Ok(record) => record,
                Err(error) => {
                    warn!(?error, "DHT record verification failed");
                    return;
                }
            };

            if step.count.get() == 1_usize {
                // This is our first record
                let results = DhtResults {
                    count: 0, // Will be increased in the next step
                    best_value: dht_record.clone(),
                    outdated_values: vec![],
                };
                event_info.state.dht_get_results.insert(id, results);
            }

            // We should always have a stored result
            let Some(results) = event_info.state.dht_get_results.get_mut(&id) else {
                log::error!(query_id = ?id, "DHT inconsistent state");
                return;
            };

            results.count += 1;
            // Replace best value if needed and update the outdated values
            if dht_record > results.best_value {
                results.outdated_values.push(results.best_value.clone());
                results.best_value = dht_record;
            } else if dht_record < results.best_value {
                results.outdated_values.push(dht_record)
            }
            // Check if we already have a quorum
            if results.count == event_info.state.dht_quorum {
                event_info
                    .swarm
                    .behaviour_mut()
                    .dht
                    .query_mut(&id)
                    .unwrap()
                    .finish();
            }
        }
        Ok(GetRecordOk::FinishedWithNoAdditionalRecord { cache_candidates }) => {
            // Remove the query, send the best result to the application layer
            // and push the best result to the cache candidates

            let Some(results) = event_info.state.dht_get_results.remove(&id) else {
                log::error!(query_id = ?id, "DHT inconsistent state");
                return;
            };

            let signed_best_record = results.best_value.clone().get_signed_record();
            // Send the best result to the application layer
            if let Some(output) = event_info.state.dht_gets.remove(&id) {
                if output.send(Ok(signed_best_record.clone().value)).is_err() {
                    error!(query_id = ?id, error = "receiver hung up", "could not send get record query result to channel");
                }
            } else {
                warn!(query_id = ?id, ?step, "GetRecord query result for unknown query ID");
            }

            if !results.outdated_values.is_empty() {
                // Now push the best value to the outdated peers
                let outdated_peers = results
                    .outdated_values
                    .iter()
                    .map(|dht_record| dht_record.get_peer_id());
                event_info.swarm.behaviour_mut().dht.put_record_to(
                    signed_best_record.clone(),
                    outdated_peers,
                    Quorum::One,
                );
            }

            // Push the best result to the cache candidates
            if !cache_candidates.is_empty() {
                let peers = cache_candidates.iter().map(|(_, &peer_id)| peer_id);
                event_info.swarm.behaviour_mut().dht.put_record_to(
                    signed_best_record,
                    peers,
                    Quorum::One,
                );
            }
        }
        Err(error) => {
            if let Some(output) = event_info.state.dht_gets.remove(&id) {
                if output.send(Err(error.clone().into())).is_err() {
                    error!(query_id = ?id, query_error=?error, error = "receiver hung up", "could not send get record query result error to channel");
                }
            } else {
                warn!(query_id = ?id, ?step, query_error=?error, "GetRecord query result error for unknown query ID");
            }
            event_info.state.dht_get_results.remove(&id);
        }
    }
}

#[cfg(feature = "kad")]
fn handle_dht_put_record(
    id: QueryId,
    result: Result<PutRecordOk, PutRecordError>,
    _stats: QueryStats,
    _step: ProgressStep,
    event_info: EventInfo,
) {
    // dht_put resolved
    if let Some(output) = event_info.state.dht_puts.remove(&id) {
        if output.send(result.map(|_| ()).map_err(Into::into)).is_err() {
            error!(query_id = ?id, error = "receiver hung up", "could not send put record query result to channel");
        }
    } else {
        warn!(query_id = ?id, "PutRecord query result for unknown query ID");
    }
}

#[cfg(feature = "kad")]
fn handle_dht_bootstrap(
    _id: QueryId,
    result: Result<BootstrapOk, BootstrapError>,
    _stats: QueryStats,
    _step: ProgressStep,
    event_info: EventInfo,
) {
    match result {
        Ok(result) => {
            if result.num_remaining != 0 {
                return;
            }
            debug!(?result, "DHT bootstrap successful");
            event_info.state.dht_bootstrap_state = DhtBootStrapState::Completed;
            if event_info.state.dht_server_mode {
                let _ = event_info.events_tx.send(NetworkEvent::DhtReady);
            }
        }
        Err(e) => error!(error = %e, "DHT bootstrap error"),
    }
}

#[cfg(feature = "kad")]
fn handle_dht_inbound_put(
    _source: PeerId,
    _connection: ConnectionId,
    record: Record,
    event_info: EventInfo,
) {
    // Verify incoming record
    let dht_record = match event_info.dht_verifier.verify(&record) {
        Ok(record) => record,
        Err(error) => {
            warn!(?error, "DHT record verification failed");
            return;
        }
    };

    // Now verify that we should overwrite it because it's better than the one we have
    let mut overwrite = true;
    let store = event_info.swarm.behaviour_mut().dht.store_mut();
    if let Some(current_record) = store.get(&record.key) {
        if let Ok(current_dht_record) = DhtRecord::try_from(&current_record.into_owned()) {
            if current_dht_record > dht_record {
                overwrite = false;
            }
        }
    }
    if overwrite && store.put(record).is_err() {
        error!("Could not store record in DHT record store");
    }
}

#[cfg(feature = "kad")]
fn handle_dht_mode_change(new_mode: Mode, event_info: EventInfo) {
    debug!(%new_mode, "DHT mode changed");
    if new_mode == Mode::Server {
        event_info.state.dht_server_mode = true;
        if event_info.state.dht_bootstrap_state == DhtBootStrapState::Completed {
            let _ = event_info.events_tx.send(NetworkEvent::DhtReady);
        }
    }
}

fn handle_discovery_event(event: discovery::Event, event_info: EventInfo) {
    event_info.swarm.behaviour_mut().pool.maintain_peers();
    match event {
        discovery::Event::Established {
            peer_id,
            peer_address,
            peer_contact,
        } => {
            let peer_info = PeerInfo::new(peer_address.clone(), peer_contact.services);

            if event_info
                .connected_peers
                .write()
                .insert(peer_id, peer_info.clone())
                .is_some()
            {
                error!(%peer_id, "Peer joined but it already exists");
                return;
            }

            info!(%peer_id, peer_address = %peer_info.get_address(), "Peer joined");
            let _ = event_info
                .events_tx
                .send(NetworkEvent::PeerJoined(peer_id, peer_info));
        }
        discovery::Event::Update => {}
    }
}

fn handle_gossipsup_event(event: gossipsub::Event, event_info: EventInfo) {
    match event {
        gossipsub::Event::Message {
            propagation_source,
            message_id,
            message,
        } => {
            #[cfg(feature = "metrics")]
            event_info
                .metrics
                .note_received_pubsub_message(&message.topic);

            let topic = message.topic.clone();

            let Some(topic_info) = event_info.state.gossip_topics.get_mut(&topic) else {
                warn!(topic = %message.topic, "unknown topic hash");
                return;
            };

            if event_info.rate_limiting.exceeds_rate_limit(
                propagation_source,
                RateLimitId::Gossipsub(topic.clone()),
                &topic_info.rate_limit_config,
            ) {
                debug!(
                    %topic,
                    peer_id = %propagation_source,
                    max_requests = %topic_info.rate_limit_config.max_requests,
                    time_window = ?topic_info.rate_limit_config.time_window,
                    "Dropping gossipsub message - rate limit exceeded",
                );
                return;
            }

            if !topic_info.validate {
                if let Err(error) = event_info
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .report_message_validation_result(
                        &message_id,
                        &propagation_source,
                        gossipsub::MessageAcceptance::Accept,
                    )
                {
                    error!(%message_id, %error, "Failed to report message validation result");
                }
            }

            if let Err(error) =
                topic_info
                    .output
                    .try_send((message, message_id, propagation_source))
            {
                error!(%topic, %error, "Failed to dispatch gossipsub message")
            }
        }
        gossipsub::Event::Subscribed { peer_id, topic } => {
            trace!(%peer_id, %topic, "peer subscribed to topic");
        }
        gossipsub::Event::Unsubscribed { peer_id, topic } => {
            trace!(%peer_id, %topic, "peer unsubscribed");
        }
        gossipsub::Event::GossipsubNotSupported { peer_id } => {
            debug!(%peer_id, "gossipsub not supported");
        }
    }
}

fn handle_ping_event(event: ping::Event, event_info: EventInfo) {
    match event.result {
        Err(error) => {
            debug!(%error, peer_id = %event.peer, "Ping failed with peer");
            event_info
                .swarm
                .behaviour_mut()
                .pool
                .close_connection(event.peer, CloseReason::RemoteClosed);
        }
        Ok(duration) => {
            trace!(?duration, peer_id = %event.peer, "Ping completed");
        }
    };
}

fn handle_request_response_event(
    event: request_response::Event<Option<Vec<u8>>, Option<Vec<u8>>>,
    event_info: EventInfo,
) {
    match event {
        request_response::Event::Message {
            peer: peer_id,
            message,
        } => match message {
            request_response::Message::Request {
                request_id,
                request,
                channel,
            } => handle_request_response_request(peer_id, request_id, request, channel, event_info),
            request_response::Message::Response {
                request_id,
                response,
            } => handle_request_response_response(peer_id, request_id, response, event_info),
        },
        request_response::Event::OutboundFailure {
            peer: peer_id,
            request_id,
            error,
        } => handle_request_response_outbound_failure(peer_id, request_id, error, event_info),
        request_response::Event::InboundFailure {
            peer: peer_id,
            request_id,
            error,
        } => handle_request_response_inbound_failure(peer_id, request_id, error, event_info),
        request_response::Event::ResponseSent { .. } => {}
    }
}

fn handle_request_response_request(
    peer_id: PeerId,
    request_id: InboundRequestId,
    request: Option<Vec<u8>>,
    channel: ResponseChannel<Option<Vec<u8>>>,
    event_info: EventInfo,
) {
    // We might get empty requests (None) because of our codec implementation
    let Some(request) = request else {
        return;
    };

    // Peek the request type, if it fails return as the request cannot be determined.
    let Ok(type_id) = peek_type(&request) else {
        debug!(%request_id, %peer_id, "Could not parse request type");
        return;
    };

    // Filter off sender if not alive.
    let sender_data = event_info
        .state
        .receive_requests
        .get(&type_id)
        .filter(|(sender, ..)| !sender.is_closed());

    // If we have a receiver, pass the request. Otherwise send a default empty response
    if let Some((sender, rate_limit_config)) = sender_data {
        if event_info.rate_limiting.exceeds_rate_limit(
            peer_id,
            RateLimitId::Request(type_id),
            rate_limit_config,
        ) {
            debug!(
                %type_id,
                %request_id,
                %peer_id,
                max_requests = %rate_limit_config.max_requests,
                time_window = ?rate_limit_config.time_window,
                "Denied request - exceeded max requests rate",
            );

            let response: Result<(), InboundRequestError> =
                Err(InboundRequestError::ExceedsRateLimit);
            if event_info
                .swarm
                .behaviour_mut()
                .request_response
                .send_response(channel, Some(response.serialize_to_vec()))
                .is_err()
            {
                error!(%type_id, %request_id, %peer_id, "Could not send rate limit error response");
            }
        } else {
            if type_id.requires_response() {
                event_info
                    .state
                    .response_channels
                    .insert(request_id, channel);
            } else {
                // Respond on behalf of the actual receiver because the actual receiver isn't interested in responding.
                let response: Result<(), InboundRequestError> = Ok(());
                if event_info
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, Some(response.serialize_to_vec()))
                    .is_err()
                {
                    error!(%type_id, %request_id, %peer_id, "Could not send auto response");
                }
            }
            if let Err(e) = sender.try_send((request.into(), request_id, peer_id)) {
                error!(%type_id, %request_id, %peer_id, error = %e, "Failed to dispatch request to handler");
            }
        }
    } else {
        trace!(%type_id, %request_id, %peer_id, "No request handler registered, replying with a 'NoReceiver' error");
        let err: Result<(), InboundRequestError> = Err(InboundRequestError::NoReceiver);
        if event_info
            .swarm
            .behaviour_mut()
            .request_response
            .send_response(channel, Some(err.serialize_to_vec()))
            .is_err()
        {
            error!(%type_id, %request_id, %peer_id, "Could not send default response");
        };

        // We remove it in case the channel was already closed.
        event_info.state.receive_requests.remove(&type_id);
    }
}

fn handle_request_response_response(
    _peer_id: PeerId,
    request_id: OutboundRequestId,
    response: Option<Vec<u8>>,
    event_info: EventInfo,
) {
    let Some(channel) = event_info.state.requests.remove(&request_id) else {
        debug!(%request_id, "No request found for response");
        return;
    };

    // We might get empty responses (None) because of the implementation of our codecs.
    let response = response
        .ok_or(RequestError::OutboundRequest(OutboundRequestError::Timeout))
        .map(|data| data.into());

    // The initiator of the request might no longer exist, so we
    // silently ignore any errors when delivering the response.
    channel.send(response).ok();

    #[cfg(feature = "metrics")]
    if let Some(instant) = event_info.state.requests_initiated.remove(&request_id) {
        event_info.metrics.note_response_time(instant.elapsed());
    }
}

fn handle_request_response_outbound_failure(
    peer_id: PeerId,
    request_id: OutboundRequestId,
    error: OutboundFailure,
    event_info: EventInfo,
) {
    error!(%request_id, %peer_id, %error, "Failed to send request to peer");

    let Some(channel) = event_info.state.requests.remove(&request_id) else {
        debug!(%request_id, %peer_id, "No request found for outbound failure");
        return;
    };

    // The request initiator might no longer exist, so silently ignore
    // any errors while delivering the response.
    channel.send(Err(to_response_error(error))).ok();
}

fn handle_request_response_inbound_failure(
    peer_id: PeerId,
    request_id: InboundRequestId,
    error: InboundFailure,
    _event_info: EventInfo,
) {
    error!(%request_id, %peer_id, %error, "Inbound request failed");
}

fn perform_action(action: NetworkAction, swarm: &mut NimiqSwarm, state: &mut TaskState) {
    match action {
        NetworkAction::Dial { peer_id, output } => {
            let dial_opts = DialOpts::peer_id(peer_id)
                .condition(PeerCondition::Disconnected)
                .build();
            let result = swarm.dial(dial_opts).map_err(Into::into);

            // The initiator might no longer exist, so we silently ignore any errors here.
            output.send(result).ok();
        }
        NetworkAction::DialAddress { address, output } => {
            let dial_opts = DialOpts::unknown_peer_id().address(address).build();
            let result = swarm.dial(dial_opts).map_err(Into::into);
            output.send(result).ok();
        }
        NetworkAction::DhtGet { key, output } => {
            #[cfg(feature = "kad")]
            let query_id = swarm.behaviour_mut().dht.get_record(key.into());
            #[cfg(feature = "kad")]
            state.dht_gets.insert(query_id, output);
        }
        NetworkAction::DhtPut { key, value, output } => {
            let local_peer_id = Swarm::local_peer_id(swarm);

            let record = Record {
                key: key.into(),
                value,
                publisher: Some(*local_peer_id),
                expires: None, // This only affects local storage. Records are replicated with configured TTL.
            };

            #[cfg(feature = "kad")]
            match swarm.behaviour_mut().dht.put_record(record, Quorum::One) {
                Ok(query_id) => {
                    // Remember put operation to resolve when we receive a `QueryResult::PutRecord`
                    state.dht_puts.insert(query_id, output);
                }
                Err(e) => {
                    output.send(Err(e.into())).ok();
                }
            }
        }
        NetworkAction::Subscribe {
            topic_name,
            buffer_size,
            validate,
            output,
            rate_limit_config,
        } => {
            let topic = gossipsub::IdentTopic::new(topic_name.clone());

            match swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                // New subscription. Insert the sender into our subscription table.
                Ok(true) => {
                    let (tx, rx) = mpsc::channel(buffer_size);

                    state.gossip_topics.insert(
                        topic.hash(),
                        GossipsubTopicInfo {
                            output: tx,
                            validate,
                            rate_limit_config,
                        },
                    );

                    let result = swarm
                        .behaviour_mut()
                        .gossipsub
                        .set_topic_params(topic, gossipsub::TopicScoreParams::default());
                    match result {
                        Ok(_) => output.send(Ok(rx)).ok(),
                        Err(e) => {
                            let error = NetworkError::TopicScoreParams {
                                topic_name: topic_name.clone(),
                                error: e,
                            };
                            output.send(Err(error)).ok()
                        }
                    };
                }

                // Apparently we're already subscribed.
                Ok(false) => {
                    let error = NetworkError::AlreadySubscribed {
                        topic_name: topic_name.clone(),
                    };
                    output.send(Err(error)).ok();
                }

                // Subscribe failed. Send back error.
                Err(e) => {
                    output.send(Err(e.into())).ok();
                }
            }
        }
        NetworkAction::Unsubscribe { topic_name, output } => {
            let topic = gossipsub::IdentTopic::new(topic_name.clone());

            if !state.gossip_topics.contains_key(&topic.hash()) {
                // If the topic wasn't in the topics list, we're not subscribed to it.
                let error = NetworkError::AlreadyUnsubscribed {
                    topic_name: topic_name.clone(),
                };
                output.send(Err(error)).ok();
                return;
            }

            match swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
                // Unsubscription. Remove the topic from the subscription table.
                Ok(true) => {
                    drop(state.gossip_topics.remove(&topic.hash()).unwrap().output);
                    output.send(Ok(())).ok();
                }

                // Apparently we're already unsubscribed.
                Ok(false) => {
                    drop(state.gossip_topics.remove(&topic.hash()).unwrap().output);
                    let error = NetworkError::AlreadyUnsubscribed {
                        topic_name: topic_name.clone(),
                    };
                    output.send(Err(error)).ok();
                }

                // Unsubscribe failed. Send back error.
                Err(e) => {
                    output.send(Err(e.into())).ok();
                }
            }
        }
        NetworkAction::Publish {
            topic_name,
            data,
            output,
        } => {
            let topic = gossipsub::IdentTopic::new(topic_name.clone());

            let result = swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic, data)
                .map(|_| ())
                .or_else(|e| match e {
                    gossipsub::PublishError::Duplicate => Ok(()),
                    _ => Err(e),
                })
                .map_err(Into::into);

            // The initiator might no longer exist, so we silently ignore any errors here.
            output.send(result).ok();
        }
        NetworkAction::NetworkInfo { output } => {
            // The initiator might no longer exist, so we silently ignore any errors here.
            output.send(Swarm::network_info(swarm)).ok();
        }
        NetworkAction::ReceiveRequests {
            type_id,
            output,
            rate_limit_config,
        } => {
            state
                .receive_requests
                .insert(type_id, (output, rate_limit_config));
        }
        NetworkAction::SendRequest {
            peer_id,
            request,
            response_channel,
            output,
        } => {
            let request_id = swarm
                .behaviour_mut()
                .request_response
                .send_request(&peer_id, Some(request));

            state.requests.insert(request_id, response_channel);
            #[cfg(feature = "metrics")]
            state.requests_initiated.insert(request_id, Instant::now());

            // The request initiator might no longer exist, so we silently ignore any errors here.
            output.send(request_id).ok();
        }
        NetworkAction::SendResponse {
            request_id,
            response,
            output,
        } => {
            let Some(response_channel) = state.response_channels.remove(&request_id) else {
                error!(%request_id, "Tried to respond to a non existing request");
                // The request initiator might no longer exist, so we silently ignore any errors here.
                output.send(Err(NetworkError::UnknownRequestId)).ok();
                return;
            };

            let result = swarm
                .behaviour_mut()
                .request_response
                .send_response(response_channel, Some(response))
                .map_err(NetworkError::ResponseChannelClosed);

            // The request initiator might no longer exist, so we silently ignore any errors here.
            output.send(result).ok();
        }
        NetworkAction::ListenOn { listen_addresses } => {
            for listen_address in listen_addresses {
                Swarm::listen_on(swarm, listen_address)
                    .expect("Failed to listen on provided address");
            }
        }
        NetworkAction::StartConnecting => {
            swarm.behaviour_mut().pool.start_connecting();
        }
        NetworkAction::ConnectPeersByServices {
            services,
            num_peers,
            output,
        } => {
            let peers_candidates = swarm
                .behaviour_mut()
                .pool
                .choose_peers_to_dial_by_services(services, num_peers);
            let mut successful_peers = vec![];

            for peer_id in peers_candidates {
                let dial_opts = DialOpts::peer_id(peer_id)
                    .condition(PeerCondition::Disconnected)
                    .build();
                if swarm.dial(dial_opts).is_ok() {
                    successful_peers.push(peer_id);
                }
            }

            output.send(successful_peers).ok();
        }
        NetworkAction::DisconnectPeer { peer_id, reason } => {
            swarm.behaviour_mut().pool.close_connection(peer_id, reason)
        }
    }
}

fn to_response_error(error: OutboundFailure) -> RequestError {
    match error {
        OutboundFailure::ConnectionClosed => {
            RequestError::OutboundRequest(OutboundRequestError::ConnectionClosed)
        }
        OutboundFailure::DialFailure => {
            RequestError::OutboundRequest(OutboundRequestError::DialFailure)
        }
        OutboundFailure::Timeout => RequestError::OutboundRequest(OutboundRequestError::Timeout),
        OutboundFailure::UnsupportedProtocols => {
            RequestError::OutboundRequest(OutboundRequestError::UnsupportedProtocols)
        }
        OutboundFailure::Io(error) => {
            RequestError::OutboundRequest(OutboundRequestError::Other(error.to_string()))
        }
    }
}
