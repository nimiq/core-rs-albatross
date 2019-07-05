use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};
use parking_lot::RwLock;

use blockchain_base::AbstractBlockchain;
use consensus::{ConsensusProtocol, Consensus};
use network_primitives::address::{PeerId, PeerUri};
use nimiq_network::address::peer_address_state::{PeerAddressInfo, PeerAddressState};
use nimiq_network::connection::close_type::CloseType;
use nimiq_network::connection::connection_info::ConnectionInfo;
use nimiq_network::connection::connection_pool::ConnectionId;
use nimiq_network::Network;
use nimiq_network::peer_scorer::Score;

use crate::handlers::Handler;
use crate::JsonRpcServerState;

pub struct NetworkHandler<P: ConsensusProtocol + 'static> {
    pub state: Arc<RwLock<JsonRpcServerState>>,
    pub network: Arc<Network<P::Blockchain>>,
    pub blockchain: Arc<P::Blockchain>,
    pub starting_block: u32,
}

impl<P: ConsensusProtocol + 'static> NetworkHandler<P> {
    pub(crate) fn new(consensus: &Arc<Consensus<P>>, state: Arc<RwLock<JsonRpcServerState>>) -> Self {
        NetworkHandler {
            state,
            network: consensus.network.clone(),
            blockchain: consensus.blockchain.clone(),
            starting_block: consensus.blockchain.head_height(),
        }
    }

    /// Returns the number of peers.
    pub(crate) fn peer_count(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(self.network.peer_count().into())
    }

    /// Returns the consensus state (string).
    pub(crate) fn consensus(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(self.state.read().consensus_state.into())
    }

    /// If syncing is true, returns an object
    /// { starting_block: number, current_block: number, highest_block: number },
    /// otherwise returns false.
    pub(crate) fn syncing(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(if self.state.read().consensus_state == "established" {
            false.into()
        }
        else {
            let current_block = self.blockchain.head_height();
            object! {
                "starting_block" => self.starting_block,
                "current_block" => current_block,
                "highest_block" => current_block // TODO
            }
        })
    }

    /// Returns a list of peer objects, each peer being described by
    /// {
    ///     id: string,
    ///     address: string,
    ///     failedAttempts: number,
    ///     addressState: number,
    ///     connectionState: number|null,
    ///     version: number|null,
    ///     timeOffset: number|null,
    ///     headHash: string|null,
    ///     score: number|null,
    ///     latency: number|null,
    ///     rx: number|null,
    ///     tx: number|null,
    /// }
    pub(crate) fn peer_list(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        let mut scores: HashMap<ConnectionId, Score> = HashMap::new();
        for (id, score) in self.network.scorer().connection_scores() {
            scores.insert(*id, *score);
        }

        Ok(self.network.addresses.state().address_info_iter()
            .map(|info| {
                let conn_id = self.network.connections.state()
                    .get_connection_id_by_peer_address(&info.peer_address);
                self.peer_address_info_to_obj(info, None,
                                              conn_id.and_then(|id| scores.get(&id)).map(|s| *s))
            })
            .collect::<Array>().into())
    }

    /// Returns the peer state for a single peer.
    /// Parameters:
    /// - uri (string): The URI for that peer.
    /// - action (string, optional): One of the possible actions `disconnect`, `fail`, `ban`, `unban`, `connect`.
    ///
    /// The return value is null or a peer object:
    /// {
    ///     id: string,
    ///     address: string,
    ///     failedAttempts: number,
    ///     addressState: number,
    ///     connectionState: number|null,
    ///     version: number|null,
    ///     timeOffset: number|null,
    ///     headHash: string|null,
    ///     score: number|null,
    ///     latency: number|null,
    ///     rx: number|null,
    ///     tx: number|null,
    /// }
    pub(crate) fn peer_state(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let peer_uri = params.get(0).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Invalid peer URI"})
            .and_then(|uri| PeerUri::from_str(uri)
                .map_err(|e| object!{"message" => e.to_string()}))?;

        let peer_id = peer_uri.peer_id()
            .ok_or_else(|| object!{"message" => "URI must contain peer ID"})
            .and_then(|s| PeerId::from_str(s)
                .map_err(|e| object!{"message" => e.to_string()}))?;

        let mut address_book = self.network.addresses.state_mut();
        let peer_address = address_book.get_by_peer_id(&peer_id)
            .ok_or_else(|| object!{"message" => "Unknown peer"})?;
        let mut peer_address_info = address_book.get_info_mut(&peer_address)
            .ok_or_else(|| object!{"message" => "Unknown peer"})?;


        let connection_pool = self.network.connections.state();
        let connection_info = connection_pool.get_connection_by_peer_address(&peer_address_info.peer_address);
        let peer_channel = connection_info.and_then(|c| c.peer_channel());

        let set = params.get(1).unwrap_or(&Null);
        if !set.is_null() {
            let set = set.as_str().ok_or_else(|| object!{"message" => "Invalid value for 'set'"})?;
            match set {
                "disconnect" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerDisconnect));
                },
                "fail" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerFail));
                },
                "ban" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerBan));
                },
                "unban" => {
                    if peer_address_info.state == PeerAddressState::Banned {
                        peer_address_info.state = PeerAddressState::Tried;
                    }
                },
                "connect" => {
                    drop(address_book);
                    drop(connection_pool);
                    self.network.connections.connect_outbound(peer_address);
                }
                _ => return Err(object!{"message" => "Unknown 'set' command."})
            }
            Ok(Null)
        }
        else {
            Ok(self.peer_address_info_to_obj(peer_address_info, connection_info, None))
        }
    }

    pub(crate) fn peer_address_info_to_obj(&self, peer_address_info: &PeerAddressInfo, connection_info: Option<&ConnectionInfo<P::Blockchain>>, score: Option<Score>) -> JsonValue {
        let state = self.network.connections.state();
        let connection_info = connection_info.or_else(|| {
            state.get_connection_by_peer_address(&peer_address_info.peer_address)
        });
        let peer = connection_info.and_then(|conn| conn.peer());

        object!{
            "id" => peer_address_info.peer_address.peer_id().to_hex(),
            "address" => peer_address_info.peer_address.as_uri().to_string(),
            "failedAttempts" => peer_address_info.failed_attempts,
            "addressState" => peer_address_info.state as u8,
            "connectionState" => connection_info.map(|conn| (conn.state() as u8).into()).unwrap_or(Null),
            "version" => peer.map(|peer| peer.version.into()).unwrap_or(Null),
            "timeOffset" => peer.map(|peer| peer.time_offset.into()).unwrap_or(Null),
            "headHash" => peer.map(|peer| peer.head_hash.to_hex().into()).unwrap_or(Null),
            "score" => score.map(|s| s.into()).unwrap_or(Null),
            "latency" => connection_info.map(|conn| conn.statistics().latency_median().into()).unwrap_or(Null),
            "rx" => Null, // TODO: Not in NetworkConnection
            "tx" => Null,
        }
    }
}

impl<P: ConsensusProtocol + 'static> Handler for NetworkHandler<P> {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>> {
        match name {
            // Network
            "peerCount" => Some(self.peer_count(params)),
            "syncing" => Some(self.syncing(params)),
            "consensus" => Some(self.consensus(params)),
            "peerList" => Some(self.peer_list(params)),
            "peerState" => Some(self.peer_state(params)),

            _ => None
        }
    }
}
