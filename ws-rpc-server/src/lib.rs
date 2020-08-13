#[macro_use]
extern crate log;

extern crate nimiq_blockchain_albatross as blockchain_albatross;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_consensus as consensus;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;
#[cfg(feature = "validator")]
extern crate nimiq_validator as validator;

use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::sink::Sink;
use futures::sync::mpsc::{channel, Sender};
use futures::{Future, IntoFuture, Stream};
use json::{object, JsonValue};
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use blockchain_albatross::BlockchainEvent;
use blockchain_base::AbstractBlockchain;
use consensus::{AlbatrossConsensusProtocol, Consensus};
use hash::{Blake2bHash, Hash};
use utils::unique_id::UniqueId;
#[cfg(feature = "validator")]
use validator::validator::Validator;
#[cfg(feature = "validator")]
use validator::validator_network::ValidatorNetworkEvent;

pub type WsRpcServerFuture = Box<dyn Future<Item = (), Error = ()> + Send + Sync + 'static>;

type WsRpcConnections = Arc<RwLock<HashMap<UniqueId, WsRpcConnection>>>;

struct WsRpcConnection {
    address: SocketAddr,
    tx: Sender<Message>,
}

pub struct WsRpcServer {
    future: WsRpcServerFuture,
    connections: WsRpcConnections,
}

impl WsRpcServer {
    const QUEUE_SIZE: usize = 64;

    pub fn new(ip: IpAddr, port: u16) -> Result<Self, IoError> {
        let socket = TcpListener::bind(&SocketAddr::new(ip, port))?;

        let connections = Arc::new(RwLock::new(HashMap::new()));
        let connections_tcp = Arc::clone(&connections);

        // Listen for incoming connections, do websocket handshake and put them in connections.
        let future = socket
            .incoming()
            .for_each(move |stream| {
                let address = stream.peer_addr().unwrap();
                let connection_id = UniqueId::new();
                // TODO: IP filter here
                info!("Client connected: {}, id={}", address, connection_id);

                let connections_stream = Arc::clone(&connections_tcp);
                let connections_err = Arc::clone(&connections_tcp);

                accept_async(stream)
                    .and_then(move |ws_stream| {
                        // Split stream
                        let (sink, stream) = ws_stream.split();

                        // Create MPSC channel
                        let (tx, rx) = channel::<Message>(Self::QUEUE_SIZE);

                        // Send everything from the MSPC channel
                        let send_future = sink.send_all(rx.map_err(|_| WsError::ConnectionClosed));

                        // Receive messages (and ignore them)
                        let connection_id_recv = connection_id;
                        let connections_recv = Arc::clone(&connections_stream);
                        let recv_future = stream.for_each(move |message: Message| {
                            // Received message. We ignore those. But we could use this for
                            // authentication and just set a flag in the connection to enable
                            // streaming events.
                            //
                            // TODO: We'll also receive close frames, which let's us close the
                            // connection gracefully.

                            // Log message
                            debug!("Received message from #{}: {}", connection_id_recv, message);

                            // Handle message
                            match message {
                                Message::Close(_close_frame_opt) => {
                                    // Remove connection from connections map
                                    let _connection = connections_recv
                                        .write()
                                        .remove(&connection_id_recv)
                                        .ok_or(WsError::AlreadyClosed)?;

                                    Ok(())
                                }
                                Message::Text(_message) => {
                                    // TODO: Handle authentication
                                    Ok(())
                                }
                                _ => {
                                    // Abort connection for everything else
                                    Err(WsError::ConnectionClosed)
                                }
                            }
                        });

                        // Put sink into connections
                        connections_stream
                            .write()
                            .insert(connection_id.clone(), WsRpcConnection { address, tx });

                        let connection_future =
                            send_future.join(recv_future).map(|_| ()).map_err(move |e| {
                                // Handle errors here. We'll just log it and close the connection.
                                warn!("Connection error: #{}: {}", connection_id, e);

                                // Close connection
                                connections_err.write().remove(&connection_id);
                            });

                        tokio::spawn(connection_future);
                        Ok(())
                    })
                    .map_err(|e| {
                        IoError::new(ErrorKind::BrokenPipe, format!("Connection error: {}", e))
                    })
            })
            .map_err(|e| {
                error!("Server socket failed: {}", e);
            });

        Ok(Self {
            future: Box::new(future),
            connections,
        })
    }

    pub fn register_blockchain(&self, consensus: Arc<Consensus<AlbatrossConsensusProtocol>>) {
        let connections_listener = Arc::clone(&self.connections);

        consensus
            .blockchain
            .register_listener(move |event: &BlockchainEvent| {
                if !connections_listener.read().is_empty() {
                    if let Some(message) = Self::map_blockchain_event(event) {
                        Self::broadcast_message(&connections_listener, message)
                    }
                }
            });
    }

    #[cfg(feature = "validator")]
    pub fn register_validator(&self, validator: Arc<Validator>) {
        let connections_listener = Arc::clone(&self.connections);

        validator.validator_network.notifier.write().register(
            move |event: &ValidatorNetworkEvent| {
                if !connections_listener.read().is_empty() {
                    if let Some(message) = Self::map_validator_event(event) {
                        Self::broadcast_message(&connections_listener, message)
                    }
                }
            },
        );
    }

    fn map_blockchain_event(event: &BlockchainEvent) -> Option<JsonValue> {
        Some(match event {
            BlockchainEvent::Extended(block_hash) => object! {
                "eventType" => "blockchainExtended",
                "blockHash" => block_hash.to_string(),
            },
            BlockchainEvent::Rebranched(reverted, rebranched) => {
                let (old_head, _) = reverted.last()?;
                let (new_head, _) = rebranched.last()?;

                let reverted = JsonValue::Array(
                    reverted
                        .into_iter()
                        .map(|(block_hash, _)| JsonValue::String(block_hash.to_string()))
                        .collect(),
                );

                let rebranched = JsonValue::Array(
                    rebranched
                        .into_iter()
                        .map(|(block_hash, _)| JsonValue::String(block_hash.to_string()))
                        .collect(),
                );

                object! {
                    "eventType" => "blockchainRebranched",
                    "reverted" => reverted,
                    "rebranched" => rebranched,
                    "oldHead" => old_head.to_string(),
                    "newHead" => new_head.to_string(),
                }
            }
            BlockchainEvent::Finalized(block_hash) => object! {
                "eventType" => "blockchainFinalized",
                "blockHash" => block_hash.to_string(),
            },
            BlockchainEvent::EpochFinalized(block_hash) => object! {
                "eventType" => "blockchainEpochFinalized",
                "blockHash" => block_hash.to_string(),
            },
        })
    }

    #[cfg(feature = "validator")]
    fn map_validator_event(event: &ValidatorNetworkEvent) -> Option<JsonValue> {
        Some(match event {
            ValidatorNetworkEvent::PbftProposal(proposal) => object! {
                "eventType" => "pbftProposal",
                "hash" => proposal.header.hash::<Blake2bHash>().to_string(),
                "blockNumber" => proposal.header.block_number,
            },
            ValidatorNetworkEvent::PbftUpdate(event) => object! {
                "eventType" => "pbftUpdate",
                "hash" => event.hash.to_string(),
                "prepareVotes" => event.prepare_votes,
                "commitVotes" => event.commit_votes,
            },
            ValidatorNetworkEvent::ViewChangeUpdate(event) => object! {
                "eventType" => "viewChangeUpdate",
                "blockNumber" => event.view_change.block_number,
                "newViewNumber" => event.view_change.new_view_number,
                "votes" => event.votes,
            },
            _ => return None,
        })
    }

    fn broadcast_message(connections: &WsRpcConnections, message: JsonValue) {
        // Convert JSON message to Websocket TEXT frame
        let message = Message::Text(message.dump());

        for (_connection_id, connection) in connections.read().iter() {
            let mut tx = connection.tx.clone();

            // If the buffer is full, we drop the event. It's not the end of the world.
            if let Err(e) = tx.try_send(message.clone()) {
                warn!("Unable to send event to {}: {}", connection.address, e);
            }
        }
    }
}

impl IntoFuture for WsRpcServer {
    type Future = WsRpcServerFuture;
    type Item = ();
    type Error = ();

    fn into_future(self) -> Self::Future {
        self.future
    }
}
