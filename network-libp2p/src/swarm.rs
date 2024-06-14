use std::{collections::HashMap, num::NonZeroU8, sync::Arc};

use base64::Engine;
use futures::StreamExt;
use instant::Instant;
#[cfg(all(target_family = "wasm", not(feature = "tokio-websocket")))]
use libp2p::websocket_websys;
use libp2p::{
    autonat::{self, OutboundFailure},
    core::{
        self,
        muxing::StreamMuxerBox,
        transport::{Boxed, MemoryTransport},
    },
    gossipsub,
    identity::Keypair,
    kad::{self, store::RecordStore, GetRecordOk, InboundRequest, QueryResult, Quorum, Record},
    noise, request_response,
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        SwarmEvent,
    },
    yamux, PeerId, Swarm, SwarmBuilder, Transport,
};
#[cfg(feature = "tokio-websocket")]
use libp2p::{dns, tcp, websocket};
use log::Instrument;
use nimiq_bls::{CompressedPublicKey, KeyPair};
use nimiq_network_interface::{
    network::{CloseReason, NetworkEvent},
    peer_info::PeerInfo,
    request::{peek_type, InboundRequestError, OutboundRequestError, RequestError},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_time::Interval;
use nimiq_utils::tagged_signing::{TaggedSignable, TaggedSigned};
use nimiq_validator_network::validator_record::ValidatorRecord;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::{
    behaviour, connection_pool,
    discovery::{behaviour::Event, peer_contacts::PeerContactBook},
    network_types::{
        DhtBootStrapState, DhtRecord, DhtResults, NetworkAction, TaskState, ValidateMessage,
    },
    rate_limiting::{remove_rate_limits, PendingDeletion, RateLimit},
    Config, NetworkError, TlsConfig,
};

type NimiqSwarm = Swarm<behaviour::Behaviour>;

pub(crate) fn new_swarm(
    config: Config,
    contacts: Arc<RwLock<PeerContactBook>>,
    peer_score_params: gossipsub::PeerScoreParams,
    force_dht_server_mode: bool,
) -> Swarm<behaviour::Behaviour> {
    let keypair = config.keypair.clone();
    let transport = new_transport(&keypair, config.memory_transport, config.tls.as_ref()).unwrap();

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
    peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
    rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
    mut update_scores: Interval,
    contacts: Arc<RwLock<PeerContactBook>>,
    force_dht_server_mode: bool,
    dht_quorum: NonZeroU8,
    #[cfg(feature = "metrics")] metrics: Arc<NetworkMetrics>,
) {
    let mut task_state = TaskState {
        dht_server_mode: force_dht_server_mode,
        dht_quorum: dht_quorum.into(),
        ..Default::default()
    };

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
                        handle_event(event, &events_tx, &mut swarm, &mut task_state, &connected_peers, Arc::clone(&peer_request_limits), Arc::clone(&rate_limits_pending_deletion), #[cfg( feature = "metrics")] &metrics);
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
    tls: Option<&TlsConfig>,
) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
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
        // Fixme: Handle wasm compatible transport

        let yamux = yamux::Config::default();

        Ok(transport
            .upgrade(core::upgrade::Version::V1)
            .authenticate(noise::Config::new(keypair).unwrap())
            .multiplex(yamux)
            .timeout(std::time::Duration::from_secs(20))
            .boxed())
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

        let yamux = yamux::Config::default();

        Ok(transport
            .upgrade(core::upgrade::Version::V1)
            .authenticate(noise::Config::new(keypair).unwrap())
            .multiplex(yamux)
            .timeout(std::time::Duration::from_secs(20))
            .boxed())
    }
}

fn handle_event(
    event: SwarmEvent<behaviour::BehaviourEvent>,
    events_tx: &broadcast::Sender<NetworkEvent<PeerId>>,
    swarm: &mut NimiqSwarm,
    state: &mut TaskState,
    connected_peers: &RwLock<HashMap<PeerId, PeerInfo>>,
    peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
    rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
    #[cfg(feature = "metrics")] metrics: &Arc<NetworkMetrics>,
) {
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
                    trace!(
                        %peer_id,
                        address = %addr,
                        %error,
                        "Removing addresses that caused dial failures",
                    );
                    swarm.behaviour_mut().remove_peer_address(peer_id, addr);
                }
            }

            // Save dialed peer addresses
            if endpoint.is_dialer() {
                let listen_addr = endpoint.get_remote_address();

                if swarm.behaviour().is_address_dialable(listen_addr) {
                    debug!(%peer_id, address = %listen_addr, "Saving peer");

                    swarm
                        .behaviour_mut()
                        .add_peer_address(peer_id, listen_addr.clone());

                    // Bootstrap Kademlia if we're performing our first connection
                    if state.dht_bootstrap_state == DhtBootStrapState::NotStarted {
                        debug!("Bootstrapping DHT");
                        if swarm.behaviour_mut().dht.bootstrap().is_err() {
                            error!("Bootstrapping DHT error: No known peers");
                        }
                        state.dht_bootstrap_state = DhtBootStrapState::Started;
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
            info!(
                %connection_id,
                %peer_id,
                ?endpoint,
                connections = num_established,
                "Connection closed with peer",
            );

            if let Some(cause) = cause {
                info!(%cause, "Connection closed because");
            }

            // Remove Peer
            if num_established == 0 {
                connected_peers.write().remove(&peer_id);
                swarm.behaviour_mut().remove_peer(peer_id);

                // Removes or marks to remove the respective rate limits.
                // Also cleans up the expired rate limits pending to delete.
                remove_rate_limits(peer_request_limits, rate_limits_pending_deletion, peer_id);

                let _ = events_tx.send(NetworkEvent::PeerLeft(peer_id));
            }
        }
        SwarmEvent::IncomingConnection {
            connection_id,
            local_addr,
            send_back_addr,
        } => {
            debug!(
                %connection_id,
                address = %send_back_addr,
                listen_address = %local_addr,
                "Incoming connection",
            );
        }

        SwarmEvent::IncomingConnectionError {
            connection_id,
            local_addr,
            send_back_addr,
            error,
        } => {
            debug!(
                %connection_id,
                address = %send_back_addr,
                listen_address = %local_addr,
                %error,
                "Incoming connection error",
            );
        }

        SwarmEvent::Dialing {
            peer_id,
            connection_id: _,
        } => {
            // This event is only triggered if the network behaviour performs the dial
            debug!(?peer_id, "Dialing peer");
        }

        SwarmEvent::NewListenAddr {
            listener_id: _,
            address,
        } => {
            debug!(%address, "New listen address");
            swarm
                .behaviour_mut()
                .discovery
                .add_own_addresses([address].to_vec());
        }

        SwarmEvent::Behaviour(event) => {
            match event {
                behaviour::BehaviourEvent::Autonat(event) => match event {
                    autonat::Event::InboundProbe(event) => {
                        log::trace!(?event, "Inbound probe");
                    }
                    autonat::Event::OutboundProbe(event) => {
                        log::trace!(?event, "Outbound probe");
                    }
                    autonat::Event::StatusChanged { old, new } => {
                        log::debug!(?old, ?new, "Autonat status changed");
                        if new == autonat::NatStatus::Private {
                            log::warn!("Couldn't detect a public reachable address. Validator network operations won't be possible");
                            log::warn!("You may need to find a relay to enable validator network operations");
                        }
                    }
                },
                behaviour::BehaviourEvent::ConnectionLimits(_) => {}
                behaviour::BehaviourEvent::Dht(event) => {
                    match event {
                        kad::Event::OutboundQueryProgressed {
                            id,
                            result,
                            stats: _,
                            step,
                        } => {
                            match result {
                                QueryResult::GetRecord(Ok(GetRecordOk::FoundRecord(record))) => {
                                    if let Some(dht_record) = verify_record(&record.record) {
                                        if step.count.get() == 1_usize {
                                            // This is our first record
                                            let results = DhtResults {
                                                count: 0, // Will be increased in the next step
                                                best_value: dht_record.clone(),
                                                outdated_values: vec![],
                                            };
                                            state.dht_get_results.insert(id, results);
                                        }
                                        // We should always have a stored result
                                        if let Some(results) = state.dht_get_results.get_mut(&id) {
                                            results.count += 1;
                                            // Replace best value if needed and update the outdated values
                                            if dht_record > results.best_value {
                                                results
                                                    .outdated_values
                                                    .push(results.best_value.clone());
                                                results.best_value = dht_record;
                                            } else if dht_record < results.best_value {
                                                results.outdated_values.push(dht_record)
                                            }
                                            // Check if we already have a quorum
                                            if results.count == state.dht_quorum {
                                                swarm
                                                    .behaviour_mut()
                                                    .dht
                                                    .query_mut(&id)
                                                    .unwrap()
                                                    .finish();
                                            }
                                        } else {
                                            log::error!(query_id = ?id, "DHT inconsistent state");
                                        }
                                    } else {
                                        warn!(
                                            "DHT record verification failed: Invalid public key received"
                                        );
                                    }
                                }
                                QueryResult::GetRecord(Ok(
                                    GetRecordOk::FinishedWithNoAdditionalRecord {
                                        cache_candidates,
                                    },
                                )) => {
                                    // Remove the query, send the best result to the application layer
                                    // and push the best result to the cache candidates
                                    if let Some(results) = state.dht_get_results.remove(&id) {
                                        let signed_best_record =
                                            results.best_value.clone().get_signed_record();
                                        // Send the best result to the application layer
                                        if let Some(output) = state.dht_gets.remove(&id) {
                                            if output
                                                .send(Ok(signed_best_record.clone().value))
                                                .is_err()
                                            {
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
                                            swarm.behaviour_mut().dht.put_record_to(
                                                signed_best_record.clone(),
                                                outdated_peers,
                                                Quorum::One,
                                            );
                                        }
                                        // Push the best result to the cache candidates
                                        if !cache_candidates.is_empty() {
                                            let peers = cache_candidates
                                                .iter()
                                                .map(|(_, &peer_id)| peer_id);
                                            swarm.behaviour_mut().dht.put_record_to(
                                                signed_best_record,
                                                peers,
                                                Quorum::One,
                                            );
                                        }
                                    } else {
                                        panic!("DHT inconsistent state, query_id: {:?}", id);
                                    }
                                }
                                QueryResult::GetRecord(Err(error)) => {
                                    if let Some(output) = state.dht_gets.remove(&id) {
                                        if output.send(Err(error.clone().into())).is_err() {
                                            error!(query_id = ?id, query_error=?error, error = "receiver hung up", "could not send get record query result error to channel");
                                        }
                                    } else {
                                        warn!(query_id = ?id, ?step, query_error=?error, "GetRecord query result error for unknown query ID");
                                    }
                                    state.dht_get_results.remove(&id);
                                }
                                QueryResult::PutRecord(result) => {
                                    // dht_put resolved
                                    if let Some(output) = state.dht_puts.remove(&id) {
                                        if output
                                            .send(result.map(|_| ()).map_err(Into::into))
                                            .is_err()
                                        {
                                            error!(query_id = ?id, error = "receiver hung up", "could not send put record query result to channel");
                                        }
                                    } else {
                                        warn!(query_id = ?id, "PutRecord query result for unknown query ID");
                                    }
                                }
                                QueryResult::Bootstrap(result) => match result {
                                    Ok(result) => {
                                        if result.num_remaining == 0 {
                                            debug!(?result, "DHT bootstrap successful");
                                            state.dht_bootstrap_state =
                                                DhtBootStrapState::Completed;
                                            if state.dht_server_mode {
                                                let _ = events_tx.send(NetworkEvent::DhtReady);
                                            }
                                        }
                                    }
                                    Err(e) => error!(error = %e, "DHT bootstrap error"),
                                },
                                _ => {}
                            }
                        }
                        kad::Event::InboundRequest {
                            request:
                                InboundRequest::PutRecord {
                                    source: _,
                                    connection: _,
                                    record: Some(record),
                                },
                        } => {
                            // Verify incoming record
                            if let Some(dht_record) = verify_record(&record) {
                                // Now verify that we should overwrite it because it's better than the one we have
                                let mut overwrite = true;
                                let store = swarm.behaviour_mut().dht.store_mut();
                                if let Some(current_record) = store.get(&record.key) {
                                    if let Ok(current_dht_record) =
                                        DhtRecord::try_from(&current_record.into_owned())
                                    {
                                        if current_dht_record > dht_record {
                                            overwrite = false;
                                        }
                                    }
                                }
                                if overwrite && store.put(record).is_err() {
                                    error!("Could not store record in DHT record store");
                                }
                            } else {
                                warn!(
                                    "DHT record verification failed: Invalid public key received"
                                );
                            }
                        }
                        kad::Event::ModeChanged { new_mode } => {
                            debug!(%new_mode, "DHT mode changed");
                            if new_mode == kad::Mode::Server {
                                state.dht_server_mode = true;
                                if state.dht_bootstrap_state == DhtBootStrapState::Completed {
                                    let _ = events_tx.send(NetworkEvent::DhtReady);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                behaviour::BehaviourEvent::Discovery(event) => {
                    swarm.behaviour_mut().pool.maintain_peers();
                    match event {
                        Event::Established {
                            peer_id,
                            peer_address,
                            peer_contact,
                        } => {
                            let peer_info =
                                PeerInfo::new(peer_address.clone(), peer_contact.services);
                            if connected_peers
                                .write()
                                .insert(peer_id, peer_info.clone())
                                .is_none()
                            {
                                info!(%peer_id, peer_address = %peer_info.get_address(), "Peer joined");
                                let _ =
                                    events_tx.send(NetworkEvent::PeerJoined(peer_id, peer_info));

                                if swarm.behaviour().is_address_dialable(&peer_address) {
                                    swarm
                                        .behaviour_mut()
                                        .add_peer_address(peer_id, peer_address);

                                    // Bootstrap Kademlia if we're adding our first address
                                    if state.dht_bootstrap_state == DhtBootStrapState::NotStarted {
                                        debug!("Bootstrapping DHT");
                                        if swarm.behaviour_mut().dht.bootstrap().is_err() {
                                            error!("Bootstrapping DHT error: No known peers");
                                        }
                                        state.dht_bootstrap_state = DhtBootStrapState::Started;
                                    }
                                }
                            } else {
                                error!(%peer_id, "Peer joined but it already exists");
                            }
                        }
                        Event::Update => {}
                    }
                }
                behaviour::BehaviourEvent::Gossipsub(event) => match event {
                    gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message,
                    } => {
                        let topic = message.topic.clone();
                        if let Some(topic_info) = state.gossip_topics.get_mut(&topic) {
                            let (output, validate) = topic_info;
                            if !&*validate {
                                if let Err(error) = swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .report_message_validation_result(
                                        &message_id,
                                        &propagation_source,
                                        gossipsub::MessageAcceptance::Accept,
                                    )
                                {
                                    error!(%message_id, %error, "could not send message validation result to channel");
                                }
                            }

                            if let Err(error) =
                                output.try_send((message, message_id, propagation_source))
                            {
                                error!(
                                    %topic,
                                    %error,
                                    "Failed to dispatch gossipsub message",
                                )
                            }
                        } else {
                            warn!(topic = %message.topic, "unknown topic hash");
                        }
                        #[cfg(feature = "metrics")]
                        metrics.note_received_pubsub_message(&topic);
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
                },
                behaviour::BehaviourEvent::Ping(event) => {
                    match event.result {
                        Err(error) => {
                            log::debug!(%error, ?event.peer, "Ping failed with peer");
                            swarm
                                .behaviour_mut()
                                .pool
                                .close_connection(event.peer, CloseReason::RemoteClosed);
                        }
                        Ok(duration) => {
                            log::trace!(?event.peer, ?duration, "Successful ping from peer");
                        }
                    };
                }
                behaviour::BehaviourEvent::Pool(event) => {
                    match event {
                        connection_pool::Event::PeerJoined => {}
                    };
                }
                behaviour::BehaviourEvent::RequestResponse(event) => match event {
                    request_response::Event::Message {
                        peer: peer_id,
                        message,
                    } => match message {
                        request_response::Message::Request {
                            request_id,
                            request,
                            channel,
                        } => {
                            // We might get empty requests (None) because of our codec implementation
                            if let Some(request) = request {
                                // TODO Add rate limiting (per peer).
                                if let Ok(type_id) = peek_type(&request) {
                                    trace!(
                                        %request_id,
                                        %peer_id,
                                        %type_id,
                                        content = &*base64::prelude::BASE64_STANDARD.encode(&request),
                                        "Incoming request from peer",
                                    );
                                    // Check if we have a receiver registered for this message type
                                    let sender = match state.receive_requests.get_mut(&type_id) {
                                        // Check if the sender is still alive, if not remove it
                                        Some(sender) if !sender.is_closed() => Some(sender),
                                        Some(_) => {
                                            state.receive_requests.remove(&type_id);
                                            None
                                        }
                                        None => None,
                                    };
                                    // If we have a receiver, pass the request. Otherwise send a default empty response
                                    if let Some(sender) = sender {
                                        if type_id.requires_response() {
                                            state.response_channels.insert(request_id, channel);
                                        } else {
                                            // Respond on behalf of the actual
                                            // receiver because the actual
                                            // receiver isn't interested in
                                            // responding.
                                            let response: Result<(), InboundRequestError> = Ok(());
                                            if swarm
                                                .behaviour_mut()
                                                .request_response
                                                .send_response(
                                                    channel,
                                                    Some(response.serialize_to_vec()),
                                                )
                                                .is_err()
                                            {
                                                error!(
                                                    %request_id,
                                                    %peer_id,
                                                    %type_id,
                                                    "Could not send auto response",
                                                );
                                            }
                                        }
                                        if let Err(e) =
                                            sender.try_send((request.into(), request_id, peer_id))
                                        {
                                            error!(
                                                %request_id,
                                                %peer_id,
                                                %type_id,
                                                error = %e,
                                                "Failed to dispatch request from peer",
                                            );
                                        }
                                    } else {
                                        trace!(
                                            %request_id,
                                            %peer_id,
                                            %type_id,
                                            "No receiver found for requests of this type, replying with a 'NoReceiver' error",
                                        );
                                        let err: Result<(), InboundRequestError> =
                                            Err(InboundRequestError::NoReceiver);
                                        if swarm
                                            .behaviour_mut()
                                            .request_response
                                            .send_response(channel, Some(err.serialize_to_vec()))
                                            .is_err()
                                        {
                                            error!(
                                                %request_id,
                                                %peer_id,
                                                %type_id,
                                                "Could not send default response",
                                            );
                                        };
                                    }
                                } else {
                                    error!(
                                        %request_id,
                                        %peer_id,
                                        content = &*base64::prelude::BASE64_STANDARD.encode(&request),
                                        "Could not parse request type",
                                    );
                                }
                            }
                        }
                        request_response::Message::Response {
                            request_id,
                            response,
                        } => {
                            trace!(
                                %request_id,
                                %peer_id,
                                "Incoming response from peer",
                            );
                            if let Some(channel) = state.requests.remove(&request_id) {
                                // We might get empty responses (None) because of the implementation of our codecs.
                                if channel
                                    .send(
                                        response
                                            .ok_or(RequestError::OutboundRequest(
                                                OutboundRequestError::Timeout,
                                            ))
                                            .map(|data| data.into()),
                                    )
                                    .is_err()
                                {
                                    error!(%request_id, %peer_id, error = "receiver hung up", "could not send response to channel");
                                }

                                #[cfg(feature = "metrics")]
                                if let Some(instant) = state.requests_initiated.remove(&request_id)
                                {
                                    metrics.note_response_time(instant.elapsed());
                                }
                            } else {
                                error!(
                                    %request_id,
                                    "No such request ID found",
                                );
                            }
                        }
                    },
                    request_response::Event::OutboundFailure {
                        peer: peer_id,
                        request_id,
                        error,
                    } => {
                        error!(
                            %request_id,
                            %peer_id,
                            %error,
                            "Failed to send request to peer",
                        );
                        if let Some(channel) = state.requests.remove(&request_id) {
                            if channel.send(Err(to_response_error(error))).is_err() {
                                error!(%request_id, %peer_id, error = "receiver hung up", "could not send outbound failure to channel");
                            }
                        } else {
                            error!(
                                %request_id,
                                %peer_id,
                                "No such request ID found"
                            );
                        }
                    }
                    request_response::Event::InboundFailure {
                        peer,
                        request_id,
                        error,
                    } => {
                        error!(
                            %request_id,
                            peer_id = %peer,
                            %error,
                            "Response to request sent from peer failed",
                        );
                    }
                    request_response::Event::ResponseSent { peer, request_id } => {
                        trace!(
                            %request_id,
                            peer_id = %peer,
                            "Response sent to peer",
                        );
                    }
                },
            }
        }
        _ => {}
    }
}

fn perform_action(action: NetworkAction, swarm: &mut NimiqSwarm, state: &mut TaskState) {
    // FIXME implement compact debug format for NetworkAction
    // trace!(?action, "performing action");

    match action {
        NetworkAction::Dial { peer_id, output } => {
            if output
                .send(
                    Swarm::dial(
                        swarm,
                        DialOpts::peer_id(peer_id)
                            .condition(PeerCondition::Disconnected)
                            .build(),
                    )
                    .map_err(Into::into),
                )
                .is_err()
            {
                error!(%peer_id, error = "receiver hung up", "could not send dial to channel");
            }
        }
        NetworkAction::DialAddress { address, output } => {
            if output
                .send(
                    Swarm::dial(swarm, DialOpts::unknown_peer_id().address(address).build())
                        .map_err(Into::into),
                )
                .is_err()
            {
                error!(error = "receiver hung up", "could not send dial to channel");
            }
        }
        NetworkAction::DhtGet { key, output } => {
            let query_id = swarm.behaviour_mut().dht.get_record(key.into());
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

            match swarm.behaviour_mut().dht.put_record(record, Quorum::One) {
                Ok(query_id) => {
                    // Remember put operation to resolve when we receive a `QueryResult::PutRecord`
                    state.dht_puts.insert(query_id, output);
                }
                Err(e) => {
                    if output.send(Err(e.into())).is_err() {
                        error!(
                            error = "receiver hung up",
                            "could not send dht put error to channel",
                        );
                    }
                }
            }
        }
        NetworkAction::Subscribe {
            topic_name,
            buffer_size,
            validate,
            output,
        } => {
            let topic = gossipsub::IdentTopic::new(topic_name.clone());

            match swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                // New subscription. Insert the sender into our subscription table.
                Ok(true) => {
                    let (tx, rx) = mpsc::channel(buffer_size);

                    state.gossip_topics.insert(topic.hash(), (tx, validate));

                    match swarm
                        .behaviour_mut()
                        .gossipsub
                        .set_topic_params(topic, gossipsub::TopicScoreParams::default())
                    {
                        Ok(_) => {
                            if output.send(Ok(rx)).is_err() {
                                error!(%topic_name, error = "receiver hung up", "could not send subscribe result to channel");
                            }
                        }
                        Err(e) => {
                            if output
                                .send(Err(NetworkError::TopicScoreParams {
                                    topic_name: topic_name.clone(),
                                    error: e,
                                }))
                                .is_err()
                            {
                                error!(%topic_name, error = "receiver hung up", "could not send subscribe error to channel");
                            }
                        }
                    };
                }

                // Apparently we're already subscribed.
                Ok(false) => {
                    if output
                        .send(Err(NetworkError::AlreadySubscribed {
                            topic_name: topic_name.clone(),
                        }))
                        .is_err()
                    {
                        error!(%topic_name, error = "receiver hung up", "could not send subscribe already subscribed error to channel");
                    }
                }

                // Subscribe failed. Send back error.
                Err(e) => {
                    if output.send(Err(e.into())).is_err() {
                        error!(%topic_name, error = "receiver hung up", "could not send subscribe error2 to channel");
                    }
                }
            }
        }
        NetworkAction::Unsubscribe { topic_name, output } => {
            let topic = gossipsub::IdentTopic::new(topic_name.clone());

            if state.gossip_topics.get_mut(&topic.hash()).is_some() {
                match swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
                    // Unsubscription. Remove the topic from the subscription table.
                    Ok(true) => {
                        drop(state.gossip_topics.remove(&topic.hash()).unwrap().0);
                        if output.send(Ok(())).is_err() {
                            error!(%topic_name, error = "receiver hung up", "could not send unsubscribe result to channel");
                        }
                    }

                    // Apparently we're already unsubscribed.
                    Ok(false) => {
                        drop(state.gossip_topics.remove(&topic.hash()).unwrap().0);
                        if output
                            .send(Err(NetworkError::AlreadyUnsubscribed {
                                topic_name: topic_name.clone(),
                            }))
                            .is_err()
                        {
                            error!(%topic_name, error = "receiver hung up", "could not send unsubscribe already unsubscribed error to channel");
                        }
                    }

                    // Unsubscribe failed. Send back error.
                    Err(e) => {
                        if output.send(Err(e.into())).is_err() {
                            error!(%topic_name, error = "receiver hung up", "could not send unsubscribe error to channel");
                        }
                    }
                }
            } else {
                // If the topic wasn't in the topics list, we're not subscribed to it.
                if output
                    .send(Err(NetworkError::AlreadyUnsubscribed {
                        topic_name: topic_name.clone(),
                    }))
                    .is_err()
                {
                    error!(%topic_name, error = "receiver hung up", "could not send unsubscribe already unsubscribed2 error to channel");
                }
            }
        }
        NetworkAction::Publish {
            topic_name,
            data,
            output,
        } => {
            let topic = gossipsub::IdentTopic::new(topic_name.clone());

            if output
                .send(
                    swarm
                        .behaviour_mut()
                        .gossipsub
                        .publish(topic, data)
                        .map(|_| ())
                        .or_else(|e| match e {
                            gossipsub::PublishError::Duplicate => Ok(()),
                            _ => Err(e),
                        })
                        .map_err(Into::into),
                )
                .is_err()
            {
                error!(%topic_name, error = "receiver hung up", "could not send publish result to channel");
            }
        }
        NetworkAction::NetworkInfo { output } => {
            if output.send(Swarm::network_info(swarm)).is_err() {
                error!(
                    error = "receiver hung up",
                    "could not send network info result to channel",
                );
            }
        }
        NetworkAction::ReceiveRequests { type_id, output } => {
            state.receive_requests.insert(type_id, output);
        }
        NetworkAction::SendRequest {
            peer_id,
            request,
            request_type_id,
            response_channel,
            output,
        } => {
            let request_id = swarm
                .behaviour_mut()
                .request_response
                .send_request(&peer_id, Some(request));
            trace!(
                %request_id,
                %peer_id,
                type_id = %request_type_id,
                "Request was sent to peer",
            );
            state.requests.insert(request_id, response_channel);
            #[cfg(feature = "metrics")]
            state.requests_initiated.insert(request_id, Instant::now());
            if output.send(request_id).is_err() {
                error!(%peer_id, %request_type_id, error = "receiver hung up", "could not send send request result to channel");
            }
        }
        NetworkAction::SendResponse {
            request_id,
            response,
            output,
        } => {
            if let Some(response_channel) = state.response_channels.remove(&request_id) {
                if output
                    .send(
                        swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(response_channel, Some(response))
                            .map_err(NetworkError::ResponseChannelClosed),
                    )
                    .is_err()
                {
                    error!(
                        %request_id,
                        error = "receiver hung up",
                        "Response was sent but the action channel was dropped",
                    );
                };
            } else {
                error!(%request_id, "Tried to respond to a non existing request");
                if output.send(Err(NetworkError::UnknownRequestId)).is_err() {
                    error!(%request_id, error = "receiver hung up", "could not send unknown request ID to channel");
                }
            }
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
                if Swarm::dial(
                    swarm,
                    DialOpts::peer_id(peer_id)
                        .condition(PeerCondition::Disconnected)
                        .build(),
                )
                .is_ok()
                {
                    successful_peers.push(peer_id);
                }
            }

            if output.send(successful_peers).is_err() {
                error!("Could not send successful peers vector");
            }
        }
        NetworkAction::DisconnectPeer { peer_id, reason } => {
            swarm.behaviour_mut().pool.close_connection(peer_id, reason)
        }
        NetworkAction::UnbanPeer { peer_id } => {
            swarm.behaviour_mut().pool.unban_connection(peer_id)
        }
    }
}

/// Returns a DHT record if the record decoding and verification was successful, None otherwise
pub(crate) fn verify_record(record: &Record) -> Option<DhtRecord> {
    if let Some(tag) = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::peek_tag(&record.value) {
        match tag {
            ValidatorRecord::<PeerId>::TAG => {
                if let Ok(validator_record) =
                    TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(
                        &record.value,
                    )
                {
                    // In this type of messages we assume the record key is also the public key used to verify these records
                    if let Ok(compressed_pk) =
                        CompressedPublicKey::deserialize_from_vec(record.key.as_ref())
                    {
                        if let Ok(pk) = compressed_pk.uncompress() {
                            if validator_record.verify(&pk) {
                                return Some(DhtRecord::Validator(
                                    record.publisher.unwrap(),
                                    validator_record.record,
                                    record.clone(),
                                ));
                            }
                        }
                    }
                }
            }
            _ => {
                log::error!(tag, "DHT invalid record tag received");
            }
        }
    }

    // If we arrived here, it's because something failed in the record verification
    None
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
