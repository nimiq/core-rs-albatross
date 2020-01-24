use std::collections::HashMap;
use std::sync::Arc;

use futures::{select, FutureExt, StreamExt};
use futures::channel::mpsc::{channel, Sender};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, Error as WsError};
use parking_lot::RwLock;
use json::{JsonValue, object};

use utils::unique_id::UniqueId;
use consensus::{Consensus, AlbatrossConsensusProtocol};
use blockchain_base::AbstractBlockchain;
use blockchain_albatross::blockchain::BlockchainEvent;
use hash::{Hash, Blake2bHash};
#[cfg(feature="validator")]
use validator::validator_network::ValidatorNetworkEvent;
#[cfg(feature="validator")]
use validator::validator::Validator;
use hyper::upgrade::Upgraded;

type WsRpcConnections = Arc<RwLock<HashMap<UniqueId, WsRpcConnection>>>;

pub(crate) struct WsRpcConnection {
    tx: Sender<Message>,
}

pub struct WsRpcServer {
    pub(crate) connections: WsRpcConnections,
}

impl WsRpcServer {
    const QUEUE_SIZE: usize = 64;

    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Moves the WebSocket stream to the connections map and spawns a message handler.
    pub(crate) async fn handle(&self, ws_stream: WebSocketStream<Upgraded>)
    {
        let connections_tcp = Arc::clone(&self.connections);

        let connections_stream = Arc::clone(&connections_tcp);
        let connections_err = Arc::clone(&connections_tcp);

        let connection_id = UniqueId::new();
        info!("Client connected: id={}", connection_id);

        // Split stream
        let (sink, mut stream) = ws_stream.split();
        // Create MPSC channel
        let (tx, rx) = channel::<Message>(Self::QUEUE_SIZE);
        let try_rx = rx.map(|x| Ok::<_, WsError>(x));
        // Send everything from the MSPC channel
        let send_future = try_rx.forward(sink);
        // Receive messages (and ignore them)
        let connection_id_recv = connection_id;
        let connections_recv = Arc::clone(&connections_stream);
        let recv_future = async move {
            while let Some(message_res) = stream.next().await {
                let message = message_res?;

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
                        let _connection = connections_recv.write().remove(&connection_id_recv)
                            .ok_or(WsError::AlreadyClosed)?;
                    },
                    Message::Text(_message) => {
                        // TODO: Handle authentication
                    },
                    _ => {
                        // Abort connection for everything else
                        return Err(WsError::ConnectionClosed);
                    }
                }
            }
            Ok(())
        };
        // Put sink into connections
        connections_stream.write()
            .insert(connection_id.clone(), WsRpcConnection {
                tx,
            });

        tokio::spawn(async move {
            let res: Result<(), _> = select! {
                        res = send_future.fuse() => res,
                        res = recv_future.fuse() => res,
                    };
            if let Err(e) = res {
                // Handle errors here. We'll just log it and close the connection.
                warn!("Connection error: #{}: {}", connection_id, e);
                // Close connection
                connections_err.write().remove(&connection_id);
            }
        });
    }

    pub fn register_blockchain(&self, consensus: Arc<Consensus<AlbatrossConsensusProtocol>>) {
        let connections_listener = Arc::clone(&self.connections);

        consensus.blockchain.register_listener(move |event: &BlockchainEvent| {
            if !connections_listener.read().is_empty() {
                if let Some(message) = Self::map_blockchain_event(event) {
                    Self::broadcast_message(&connections_listener, message)
                }
            }
        });
    }

    #[cfg(feature="validator")]
    pub fn register_validator(&self, validator: Arc<Validator>) {
        let connections_listener = Arc::clone(&self.connections);

        validator.validator_network.notifier.write().register(move |event: &ValidatorNetworkEvent| {
            if !connections_listener.read().is_empty() {
                if let Some(message) = Self::map_validator_event(event) {
                    Self::broadcast_message(&connections_listener, message)
                }
            }
        });
    }

    fn map_blockchain_event(event: &BlockchainEvent) -> Option<JsonValue> {
        Some(match event {
            BlockchainEvent::Extended(block_hash) => object!{
                "eventType" => "blockchainExtended",
                "blockHash" => block_hash.to_string(),
            },
            BlockchainEvent::Rebranched(reverted, rebranched) => {
                let (old_head, _) = reverted.last()?;
                let (new_head, _) = rebranched.last()?;

                let reverted = JsonValue::Array(reverted.into_iter()
                    .map(|(block_hash, _)|  JsonValue::String(block_hash.to_string()))
                    .collect());

                let rebranched = JsonValue::Array(rebranched.into_iter()
                    .map(|(block_hash, _)|  JsonValue::String(block_hash.to_string()))
                    .collect());

                object!{
                    "eventType" => "blockchainRebranched",
                    "reverted" => reverted,
                    "rebranched" => rebranched,
                    "oldHead" => old_head.to_string(),
                    "newHead" => new_head.to_string(),
                }
            },
            BlockchainEvent::Finalized(block_hash) => object!{
                "eventType" => "blockchainFinalized",
                "blockHash" => block_hash.to_string(),
            },
        })
    }

    #[cfg(feature="validator")]
    fn map_validator_event(event: &ValidatorNetworkEvent) -> Option<JsonValue> {
        Some(match event {
            ValidatorNetworkEvent::PbftProposal(proposal) => object!{
                "eventType" => "pbftProposal",
                "hash" => proposal.header.hash::<Blake2bHash>().to_string(),
                "blockNumber" => proposal.header.block_number,
            },
            ValidatorNetworkEvent::PbftUpdate(event) => object!{
                "eventType" => "pbftUpdate",
                "hash" => event.hash.to_string(),
                "prepareVotes" => event.prepare_votes,
                "commitVotes" => event.commit_votes,
            },
            ValidatorNetworkEvent::ViewChangeUpdate(event) => object!{
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

        for (connection_id, connection) in connections.read().iter() {
            let mut tx = connection.tx.clone();

            // If the buffer is full, we drop the event. It's not the end of the world.
            if let Err(e) = tx.try_send(message.clone()) {
                warn!("Unable to send event to {}: {}", connection_id, e);
            }
        }
    }
}
