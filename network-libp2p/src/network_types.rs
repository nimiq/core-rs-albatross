use std::collections::HashMap;

use bytes::Bytes;
#[cfg(feature = "metrics")]
use instant::Instant;
use libp2p::{
    gossipsub,
    kad::{QueryId, Record},
    request_response::{InboundRequestId, OutboundRequestId, ResponseChannel},
    swarm::NetworkInfo,
    Multiaddr, PeerId,
};
use nimiq_bls::KeyPair;
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, PubsubId, Topic},
    peer_info::Services,
    request::{RequestError, RequestType},
};
use nimiq_serde::{Deserialize, DeserializeError};
use nimiq_utils::tagged_signing::{TaggedSignable, TaggedSigned};
use nimiq_validator_network::validator_record::ValidatorRecord;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::{
    dispatch::codecs::{IncomingRequest, OutgoingResponse},
    rate_limiting::RequestRateLimitData,
    NetworkError,
};

#[derive(Debug)]
pub(crate) enum NetworkAction {
    Dial {
        peer_id: PeerId,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    DialAddress {
        address: Multiaddr,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    DhtGet {
        key: Vec<u8>,
        output: oneshot::Sender<Result<Vec<u8>, NetworkError>>,
    },
    DhtPut {
        key: Vec<u8>,
        value: Vec<u8>,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    Subscribe {
        topic_name: String,
        buffer_size: usize,
        validate: bool,
        output: oneshot::Sender<
            Result<
                mpsc::Receiver<(gossipsub::Message, gossipsub::MessageId, PeerId)>,
                NetworkError,
            >,
        >,
    },
    Unsubscribe {
        topic_name: String,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    Publish {
        topic_name: String,
        data: Vec<u8>,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    NetworkInfo {
        output: oneshot::Sender<NetworkInfo>,
    },
    ReceiveRequests {
        type_id: RequestType,
        output: mpsc::Sender<(Bytes, InboundRequestId, PeerId)>,
        request_rate_limit_data: RequestRateLimitData,
    },
    SendRequest {
        peer_id: PeerId,
        request: IncomingRequest,
        request_type_id: RequestType,
        response_channel: oneshot::Sender<Result<Bytes, RequestError>>,
        output: oneshot::Sender<OutboundRequestId>,
    },
    SendResponse {
        request_id: InboundRequestId,
        response: OutgoingResponse,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    ListenOn {
        listen_addresses: Vec<Multiaddr>,
    },
    ConnectPeersByServices {
        services: Services,
        num_peers: usize,
        output: oneshot::Sender<Vec<PeerId>>,
    },
    StartConnecting,
    DisconnectPeer {
        peer_id: PeerId,
        reason: CloseReason,
    },
}

pub(crate) struct ValidateMessage<P: Clone> {
    pub(crate) pubsub_id: GossipsubId<P>,
    pub(crate) acceptance: gossipsub::MessageAcceptance,
    pub(crate) topic: &'static str,
}

impl<P: Clone> ValidateMessage<P> {
    pub fn new<T>(pubsub_id: GossipsubId<P>, acceptance: MsgAcceptance) -> Self
    where
        T: Topic + Sync,
    {
        Self {
            pubsub_id,
            acceptance: match acceptance {
                MsgAcceptance::Accept => gossipsub::MessageAcceptance::Accept,
                MsgAcceptance::Ignore => gossipsub::MessageAcceptance::Ignore,
                MsgAcceptance::Reject => gossipsub::MessageAcceptance::Reject,
            },
            topic: <T as Topic>::NAME,
        }
    }
}

/// DHT bootstrap state
#[derive(Default, PartialEq)]
pub(crate) enum DhtBootStrapState {
    /// DHT bootstrap has been started
    #[default]
    NotStarted,
    /// DHT bootstrap has been started
    Started,
    /// DHT bootstrap has been completed
    Completed,
}

/// Enum over all of the possible DHT records values
#[derive(Clone, PartialEq)]
pub(crate) enum DhtRecord {
    /// Validator record with its publisher Peer ID,
    /// the decoded validator record and the original serialized record.
    Validator(PeerId, ValidatorRecord<PeerId>, Record),
}

impl DhtRecord {
    pub(crate) fn get_signed_record(self) -> Record {
        match self {
            Self::Validator(_, _, signed_record) => signed_record,
        }
    }

    pub(crate) fn get_peer_id(&self) -> PeerId {
        match self {
            Self::Validator(peer_id, _, _) => *peer_id,
        }
    }

    pub(crate) fn get_timestamp(&self) -> u64 {
        match self {
            Self::Validator(_, record, _) => record.timestamp,
        }
    }
}

impl PartialOrd for DhtRecord {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.get_timestamp().partial_cmp(&other.get_timestamp())
    }
}

/// DHT record decoding errors
#[derive(Debug, Error)]
pub(crate) enum DhtRecordError {
    /// Tag is unknown
    #[error("Unknown record tag")]
    UnknownTag,
    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializeError(#[from] DeserializeError),
}

impl TryFrom<&Record> for DhtRecord {
    type Error = DhtRecordError;
    fn try_from(record: &Record) -> Result<Self, Self::Error> {
        if let Some(tag) = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::peek_tag(&record.value)
        {
            match tag {
                ValidatorRecord::<PeerId>::TAG => {
                    let validator_record =
                        TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(
                            &record.value,
                        )?;
                    {
                        Ok(DhtRecord::Validator(
                            record.publisher.unwrap(),
                            validator_record.record,
                            record.clone(),
                        ))
                    }
                }
                _ => Err(DhtRecordError::UnknownTag),
            }
        } else {
            Err(DhtRecordError::UnknownTag)
        }
    }
}

/// DHT results obtained for a specific query ID
pub(crate) struct DhtResults {
    /// Number of records obtained
    pub(crate) count: u8,
    /// Best value obtained so far
    pub(crate) best_value: DhtRecord,
    /// Other (outdated) values obtained
    pub(crate) outdated_values: Vec<DhtRecord>,
}

#[derive(Default)]
pub(crate) struct TaskState {
    /// Senders for DHT (kad) put operations
    pub(crate) dht_puts: HashMap<QueryId, oneshot::Sender<Result<(), NetworkError>>>,
    /// Senders for DHT (kad) get operations
    pub(crate) dht_gets: HashMap<QueryId, oneshot::Sender<Result<Vec<u8>, NetworkError>>>,
    /// Get results for DHT (kad) get operation
    pub(crate) dht_get_results: HashMap<QueryId, DhtResults>,
    /// Senders per Gossibsub topic
    pub(crate) gossip_topics: HashMap<
        gossipsub::TopicHash,
        (
            mpsc::Sender<(gossipsub::Message, gossipsub::MessageId, PeerId)>,
            bool,
        ),
    >,
    /// DHT (kad) has been bootstrapped
    pub(crate) dht_bootstrap_state: DhtBootStrapState,
    /// DHT (kad) is in server mode
    pub(crate) dht_server_mode: bool,
    /// Senders per `OutboundRequestId` for request-response
    pub(crate) requests: HashMap<OutboundRequestId, oneshot::Sender<Result<Bytes, RequestError>>>,
    /// Time spent per `OutboundRequestId` for request-response
    #[cfg(feature = "metrics")]
    pub(crate) requests_initiated: HashMap<OutboundRequestId, Instant>,
    /// Senders for receiving responses per `InboundRequestId` for request-response
    pub(crate) response_channels:
        HashMap<InboundRequestId, ResponseChannel<Option<OutgoingResponse>>>,
    /// Senders and respective rate limiting constants for replying to requests per `RequestType` for request-response
    pub(crate) receive_requests: HashMap<
        RequestType,
        (
            mpsc::Sender<(Bytes, InboundRequestId, PeerId)>,
            RequestRateLimitData,
        ),
    >,
    /// DHT quorum value
    pub(crate) dht_quorum: u8,
}

#[derive(Clone, Debug)]
pub struct GossipsubId<P: Clone> {
    pub(crate) message_id: gossipsub::MessageId,
    pub(crate) propagation_source: P,
}

impl PubsubId<PeerId> for GossipsubId<PeerId> {
    fn propagation_source(&self) -> PeerId {
        self.propagation_source
    }
}
