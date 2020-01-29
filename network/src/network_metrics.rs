use std::{fmt, fmt::Display};
use std::collections::HashMap;
use std::default::Default;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use blockchain_base::AbstractBlockchain;
use network_messages::MessageType;
use network_primitives::protocol::Protocol;

use crate::connection::connection_info::ConnectionState;
use crate::connection::connection_pool::ConnectionPool;

#[derive(Default, Debug)]
pub struct NetworkMetrics {
    bytes_received: AtomicUsize,
    bytes_sent: AtomicUsize,
}

impl NetworkMetrics {
    #[inline]
    pub fn new(bytes_received: usize, bytes_sent: usize) -> Self {
        NetworkMetrics {
            bytes_received: AtomicUsize::new(bytes_received),
            bytes_sent: AtomicUsize::new(bytes_sent),
        }
    }

    #[inline]
    pub fn note_bytes_received(&self, bytes: usize) {
        self.bytes_received.fetch_add(bytes, Ordering::Release);
    }

    #[inline]
    pub fn bytes_received(&self) -> usize {
        self.bytes_received.load(Ordering::Acquire)
    }

    #[inline]
    pub fn note_bytes_sent(&self, bytes: usize) {
        self.bytes_sent.fetch_add(bytes, Ordering::Release);
    }

    #[inline]
    pub fn bytes_sent(&self) -> usize {
        self.bytes_sent.load(Ordering::Acquire)
    }
}

#[derive(Default)]
pub struct MessageStatistics {
    occurrences: AtomicUsize,
    processing_time: AtomicUsize,
}

#[derive(Default)]
pub struct MessageMetrics {
    messages: HashMap<MessageType, MessageStatistics>,
}

impl MessageMetrics {
    // New message types need to be added here to occur in the metrics!
    const MESSAGE_TYPES: [MessageType; 43] = [
        MessageType::Version,
        MessageType::Inv,
        MessageType::GetData,
        MessageType::GetHeader,
        MessageType::NotFound,
        MessageType::GetBlocks,
        MessageType::Block,
        MessageType::BlockAlbatross,
        MessageType::Header,
        MessageType::HeaderAlbatross,
        MessageType::Tx,
        MessageType::Mempool,
        MessageType::Reject,
        MessageType::Subscribe,
        MessageType::Addr,
        MessageType::GetAddr,
        MessageType::Ping,
        MessageType::Pong,
        MessageType::Signal,
        MessageType::GetChainProof,
        MessageType::ChainProof,
        MessageType::GetAccountsProof,
        MessageType::AccountsProof,
        MessageType::GetAccountsTreeChunk,
        MessageType::AccountsTreeChunk,
        MessageType::GetTransactionsProof,
        MessageType::TransactionsProof,
        MessageType::GetTransactionReceipts,
        MessageType::TransactionReceipts,
        MessageType::GetBlockProof,
        MessageType::BlockProof,
        MessageType::GetHead,
        MessageType::Head,
        MessageType::VerAck,
        MessageType::BlockAlbatross,
        MessageType::HeaderAlbatross,
        MessageType::ViewChange,
        MessageType::ViewChangeProof,
        MessageType::ForkProof,
        MessageType::ValidatorInfo,
        MessageType::PbftProposal,
        MessageType::PbftPrepare,
        MessageType::PbftCommit,
    ];

    pub fn new() -> Self {
        let mut metrics = MessageMetrics {
            messages: HashMap::new(),
        };

        // We prefill our datastructure here.
        for &ty in Self::MESSAGE_TYPES.iter() {
            metrics.messages.insert(ty, MessageStatistics::default());
        }

        metrics
    }

    pub fn from_map(map: HashMap<MessageType, (usize, usize)>) -> Self {
        let mut metrics = MessageMetrics {
            messages: HashMap::new(),
        };

        // We prefill our datastructure here.
        for (&k, &v) in map.iter() {
            let occurrences = AtomicUsize::new(v.0);
            let processing_time = AtomicUsize::new(v.1);
            metrics.messages.insert(k, MessageStatistics{occurrences, processing_time});
        }

        metrics
    }

    #[inline]
    pub fn note_message(&self, ty: MessageType, time: usize) {
        if let Some(msg_stats) = self.messages.get(&ty) {
            msg_stats.occurrences.fetch_add(1, Ordering::Release);
            msg_stats.processing_time.fetch_add(time, Ordering::Release);
        } else {
            warn!("Message type {:?} is not implemented in metrics!", ty);
        }
    }

    #[inline]
    pub fn message_types(&self) -> impl Iterator<Item=&MessageType> {
        self.messages.keys()
    }

    #[inline]
    pub fn message_occurrences(&self, ty: MessageType) -> Option<usize> {
        let occurrences = &self.messages.get(&ty)?.occurrences;
        Some(occurrences.load(Ordering::Acquire))
    }

    #[inline]
    pub fn message_processing_time(&self, ty: MessageType) -> Option<usize> {
        let processing_time = &self.messages.get(&ty)?.processing_time;
        Some(processing_time.load(Ordering::Acquire))
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Hash)]
#[repr(u8)]
pub enum PeerProtocol {
    Dumb,
    Wss,
    Rtc,
    Ws,
    Unknown,
}

impl Display for PeerProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(match self {
            PeerProtocol::Dumb => "dumb",
            PeerProtocol::Wss => "websocket-secure",
            PeerProtocol::Ws => "websocket",
            PeerProtocol::Rtc => "webrtc",
            PeerProtocol::Unknown => "unknown",
        })
    }
}

impl From<Protocol> for PeerProtocol {
    fn from(protocol: Protocol) -> Self {
        match protocol {
            Protocol::Dumb => PeerProtocol::Dumb,
            Protocol::Ws => PeerProtocol::Ws,
            Protocol::Wss => PeerProtocol::Wss,
            Protocol::Rtc => PeerProtocol::Rtc,
        }
    }
}

impl From<Option<Protocol>> for PeerProtocol {
    fn from(protocol: Option<Protocol>) -> Self {
        match protocol {
            Some(p) => PeerProtocol::from(p),
            None => PeerProtocol::Unknown,
        }
    }
}

#[derive(Default)]
pub struct PeerMetrics {
    peers: HashMap<(PeerProtocol, ConnectionState), usize>,
}

impl PeerMetrics {
    fn add_peer<P: Into<PeerProtocol>>(&mut self, protocol: P, state: ConnectionState) {
        let protocol: PeerProtocol = protocol.into();
        *self.peers.entry((protocol, state))
            .or_insert(0) += 1;
    }

    pub fn peer_metrics(&self) -> impl Iterator<Item=(&(PeerProtocol, ConnectionState), &usize)> {
        self.peers.iter()
    }
}

impl<B: AbstractBlockchain + 'static> ConnectionPool<B> {
    pub fn metrics(&self) -> (MessageMetrics, NetworkMetrics, PeerMetrics) {
        let mut bytes_sent: usize;
        let mut bytes_received: usize;
        let mut peer_metrics = PeerMetrics::default();
        // We count the message metrics afterwards to minimize time of locking state.
        let mut message_metrics: Vec<Arc<MessageMetrics>> = Vec::new();
        let mut messages: HashMap<MessageType, (usize, usize)> = HashMap::new();

        // Connection pool state lock.
        {
            let state = self.state();
            let past_conn_metrics = state.get_past_conn_metrics();

            bytes_sent = past_conn_metrics.bytes_sent();
            bytes_received = past_conn_metrics.bytes_received();

            for connection in state.connection_iter() {
                // Copy over message metrics.
                if let Some(channel) = connection.peer_channel() {
                    message_metrics.push(channel.message_metrics.clone());
                }

                // Retrieve network stats.
                if let Some(conn) = connection.network_connection() {
                    let metrics = conn.metrics();
                    bytes_sent += metrics.bytes_sent();
                    bytes_received += metrics.bytes_received();
                }

                // Collect peer information.
                let protocol = connection.peer_address().map(|addr| addr.protocol());
                peer_metrics.add_peer(protocol, connection.state());
            }
        }

        // Construct message metrics.
        for m in message_metrics.iter() {
            for &ty in m.message_types() {
                let entry = messages.entry(ty).or_insert((0,0));
                entry.0 += m.message_occurrences(ty).unwrap_or(0);
                entry.1 += m.message_processing_time(ty).unwrap_or(0);
            }
        }

        (MessageMetrics::from_map(messages), NetworkMetrics::new(bytes_received, bytes_sent), peer_metrics)
    }
}
