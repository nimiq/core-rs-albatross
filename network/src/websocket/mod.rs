use std::{collections::VecDeque, fmt, fmt::Debug, io, net, time::Instant};
#[cfg(feature = "metrics")]
use std::sync::Arc;

use futures::prelude::*;
use tokio::{
    net::TcpStream,
};
use tokio_tungstenite::{
    accept_hdr_async,
    connect_async,
    MaybeTlsStream,
    stream::PeerAddr,
    WebSocketStream
};
use tungstenite::{
    error::Error as WsError,
    protocol::Message as WebSocketMessage
};
use tungstenite::handshake::server::Callback;
use tungstenite::protocol::CloseFrame;
use url::Url;

use beserial::{Deserialize, Serialize, SerializingError};
use network_messages::Message as NimiqMessage;
use network_primitives::address::net_address::NetAddress;
use utils::locking::MultiLock;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;

pub mod websocket_connector;
mod reverse_proxy;

type WebSocketLayer = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub trait IntoData {
    fn into_data(self) -> Vec<u8>;
}

impl IntoData for WebSocketMessage {
    fn into_data(self) -> Vec<u8> {
        self.into_data()
    }
}

#[derive(Debug)]
pub enum NimiqMessageStreamError {
    WebSocketError(WsError),
    TagMismatch,
    ParseError(SerializingError),
    ChunkSizeExceeded,
    MessageSizeExceeded,
    FinalChunkSizeExceeded,
}

pub enum NimiqWebSocketMessage {
    Message(NimiqMessage),
    Close(Option<CloseFrame<'static>>),
}

#[derive(Clone, Debug)]
pub enum NimiqWebSocketState {
    /// The connection is active.
    Active,
    /// We initiated a close handshake.
    ClosedByUs,
    /// The peer initiated a close handshake.
    ClosedByPeer(Option<CloseFrame<'static>>),
}

impl NimiqWebSocketState {
    #[inline]
    pub fn is_active(&self) -> bool {
        match self {
            NimiqWebSocketState::Active => true,
            _ => false,
        }
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        !self.is_active()
    }
}

/// This struct stores public information about the stream.
#[derive(Clone, Debug)]
pub struct PublicStreamState {
    // Constant info.
    net_address: NetAddress,
    outbound: bool,

    // Mutable info.
    last_chunk_received_at: Option<Instant>,

    #[cfg(feature = "metrics")]
    network_metrics: Arc<NetworkMetrics>,
}

impl<'a> From<&'a PublicStreamState> for PublicStreamState {
    fn from(state: &'a PublicStreamState) -> Self {
        state.clone()
    }
}

impl<'a> From<&'a NimiqMessageStream> for PublicStreamState {
    fn from(stream: &'a NimiqMessageStream) -> Self {
        stream.public_state.clone()
    }
}

impl PublicStreamState {
    pub fn new(net_address: NetAddress, outbound: bool) -> Self {
        PublicStreamState {
            net_address,
            outbound,
            last_chunk_received_at: None,

            #[cfg(feature = "metrics")]
            network_metrics: Arc::new(NetworkMetrics::default()),
        }
    }
    pub fn update(&mut self, state: &PublicStreamState) {
        self.last_chunk_received_at = state.last_chunk_received_at.clone();
    }
}

const MAX_CHUNK_SIZE: usize = 1024 * 16; // 16 kb
const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 10; // 10 mb

pub struct NimiqMessageStream {
    // Internal state.
    inner: WebSocketLayer,
    receiving_tag: u8,
    sending_tag: u8,
    ws_queue: VecDeque<WebSocketMessage>,
    msg_buf: Option<Vec<u8>>,
    state: NimiqWebSocketState,

    // Public state.
    public_state: PublicStreamState,
}

impl NimiqMessageStream {
    fn new(ws_socket: WebSocketStream<MaybeTlsStream<TcpStream>>, outbound: bool) -> Self {
        let peer_addr = ws_socket.get_ref().peer_addr().unwrap();
        return NimiqMessageStream {
            inner: ws_socket,
            receiving_tag: 254,
            sending_tag: 0,
            ws_queue: VecDeque::new(),
            msg_buf: None,
            state: NimiqWebSocketState::Active,

            public_state: PublicStreamState::new(match peer_addr.ip() {
                net::IpAddr::V4(ip4) => NetAddress::IPv4(ip4),
                net::IpAddr::V6(ip6) => NetAddress::IPv6(ip6),
            }, outbound),
        };
    }

    pub fn state(&self) -> &PublicStreamState {
        &self.public_state
    }

    pub fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    #[cfg(feature = "metrics")]
    pub fn network_metrics(&self) -> &Arc<NetworkMetrics> {
        &self.public_state.network_metrics
    }
}

impl Sink for NimiqMessageStream {
    type SinkItem = NimiqWebSocketMessage;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        // Handle close messages differently:
        let msg = match item {
            NimiqWebSocketMessage::Message(msg) => msg,
            NimiqWebSocketMessage::Close(frame) => {
                self.state = NimiqWebSocketState::ClosedByUs;

                return match self.inner.start_send(WebSocketMessage::Close(frame)) {
                    Ok(state) => match state {
                        AsyncSink::Ready => Ok(AsyncSink::Ready),
                        AsyncSink::NotReady(WebSocketMessage::Close(frame)) => Ok(AsyncSink::NotReady(NimiqWebSocketMessage::Close(frame))),
                        AsyncSink::NotReady(_) => {
                            error!("Expected to get NotReady of a Close message, but got something else.");
                            Err(())
                        },
                    },
                    Err(_) => Err(()),
                }
            },
        };

        // Save and increment tag.
        let tag = self.sending_tag;
        // XXX JS implementation quirk: Already wrap at 255 instead of 256
        self.sending_tag = (self.sending_tag + 1) % 255;

        let serialized_msg = msg.serialize_to_vec();

        #[cfg(feature = "metrics")]
        self.public_state.network_metrics.note_bytes_sent(serialized_msg.len());

        // Send chunks to underlying layer.
        let mut remaining = serialized_msg.len();
        let mut chunk;
        while remaining > 0 {
            let mut buffer;
            let start = serialized_msg.len() - remaining;
            if remaining + /*tag*/ 1 >= MAX_CHUNK_SIZE {
                buffer = Vec::with_capacity(MAX_CHUNK_SIZE + /*tag*/ 1);
                buffer.push(tag);
                chunk = &serialized_msg[start..start + MAX_CHUNK_SIZE - 1];
            } else {
                buffer = Vec::with_capacity(remaining + /*tag*/ 1);
                buffer.push(tag);
                chunk = &serialized_msg[start..];
            }

            buffer.extend(chunk);

            match self.inner.start_send(WebSocketMessage::binary(buffer)) {
                Ok(state) => match state {
                    AsyncSink::Ready => (),
                    // We started to send some chunks, but now the queue is full:
                    // FIXME If this happens, we will try sending the whole message again with a new tag.
                    // This should be improved, e.g. using https://docs.rs/futures/0.2.1/futures/sink/struct.Buffer.html.
                    AsyncSink::NotReady(_) => return Ok(AsyncSink::NotReady(NimiqWebSocketMessage::Message(msg))),
                },
                Err(_) => return Err(()),
            };

            remaining -= chunk.len();
        }
        // We didn't exit previously, so everything worked out.
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_complete() {
            Ok(r_async) => Ok(r_async),
            Err(_) => Err(()),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.close() {
            Ok(r_async) => Ok(r_async),
            Err(_) => Err(()),
        }
    }
}

impl Stream for NimiqMessageStream {
    type Item = NimiqWebSocketMessage;
    type Error = NimiqMessageStreamError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.state.is_closed() {
            return Ok(Async::Ready(None));
        }

        // First, lets get as many WebSocket messages as available and store them in the buffer.
        loop {
            match self.inner.poll() {
                // Handle close frames first.
                Ok(Async::Ready(Some(WebSocketMessage::Close(frame)))) => {
                    self.state = NimiqWebSocketState::ClosedByPeer(frame.clone());

                    return Ok(Async::Ready(Some(NimiqWebSocketMessage::Close(frame))))
                },
                Ok(Async::Ready(Some(m))) => {
                    #[cfg(feature = "metrics")]
                    self.public_state.network_metrics.note_bytes_received(m.len());

                    // Check max chunk size.
                    if m.len() > MAX_CHUNK_SIZE {
                        error!("Max chunk size exceeded ({} > {})", m.len(), MAX_CHUNK_SIZE);
                        return Err(NimiqMessageStreamError::ChunkSizeExceeded);
                    }
                    self.ws_queue.push_back(m)
                },
                Ok(Async::Ready(None)) => {
                    // FIXME: first flush our buffer and _then_ signal that there will be no more messages available
                    return Ok(Async::Ready(None))
                },
                Ok(Async::NotReady) => {
                    break
                },
                Err(e) => {
                    if let WsError::ConnectionClosed(ref frame) = e {
                        self.state = NimiqWebSocketState::ClosedByPeer(frame.clone());
                    }
                    // FIXME: first flush our buffer and _then_ signal that there was an error
                    return Err(NimiqMessageStreamError::WebSocketError(e))
                }
            }
        }

        // If there are no web socket messages in the buffer, signal that we don't have anything yet
        // (i.e. we would need to block waiting, which is a no no in an async function)
        if self.ws_queue.len() == 0 {
            return Ok(Async::NotReady);
        }

        while let Some(ws_msg) = self.ws_queue.pop_front() {
            let raw_msg = ws_msg.into_data();
            let tag = raw_msg[0];
            let chunk = &raw_msg[1..];

            // Detect if this is a new message.
            if self.msg_buf.is_none() {
                let msg_size = NimiqMessage::peek_length(chunk);
                if msg_size > MAX_MESSAGE_SIZE {
                    error!("Max message size exceeded ({} > {})", msg_size, MAX_MESSAGE_SIZE);
                    return Err(NimiqMessageStreamError::MessageSizeExceeded);
                }

                self.msg_buf = Some(Vec::with_capacity(msg_size));
                // XXX JS implementation quirk: Already wrap at 255 instead of 256
                self.receiving_tag = (self.receiving_tag + 1) % 255;
            }

            if self.receiving_tag != tag {
                error!("Tag mismatch: expected {}, got {}", self.receiving_tag, tag);
                return Err(NimiqMessageStreamError::TagMismatch);
            }

            // Update last chunk timestamp
            self.public_state.last_chunk_received_at = Some(Instant::now());

            let msg_buf = self.msg_buf.as_mut().unwrap();
            let mut remaining = msg_buf.capacity() - msg_buf.len();

            let chunk_size = raw_msg.len() - 1;
            if chunk_size > remaining {
                error!("Final chunk size exceeded ({} > {})", chunk_size, remaining);
                return Err(NimiqMessageStreamError::FinalChunkSizeExceeded);
            }

            msg_buf.extend_from_slice(chunk);
            remaining -= chunk_size;

            if remaining == 0 {
                // Full message read, parse it.
                let msg = Deserialize::deserialize(&mut &msg_buf[..]);

                // Reset message buffer.
                self.msg_buf = None;

                if let Err(e) = msg {
                    error!("Failed to parse message: {:?}", e);
                    // FIXME Fail on message parse errors
                    return Ok(Async::NotReady);
                }

                return Ok(Async::Ready(Some(NimiqWebSocketMessage::Message(msg.unwrap()))));
            }
        }

        return Ok(Async::NotReady);
    }
}

impl Debug for NimiqMessageStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "NimiqMessageStream {{}}")
    }
}

/// Connect to a given URL and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_connect_async(url: Url) -> Box<Future<Item = NimiqMessageStream, Error = io::Error> + Send> {
    Box::new(
        connect_async(url).map(|(ws_stream,_)| NimiqMessageStream::new(ws_stream, true))
        .map_err(|e| {
            println!("Error while trying to connect to another node: {}", e);
            io::Error::new(io::ErrorKind::Other, e)
        })
    )
}

/// Accept an incoming connection and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_accept_async<C>(stream: MaybeTlsStream<TcpStream>, callback: C) -> Box<Future<Item = NimiqMessageStream, Error = io::Error> + Send>
    where C: Callback + Send + 'static { // TODO: Why do we need 'static here?!
    Box::new(
        accept_hdr_async(stream, callback).map(|ws_stream| NimiqMessageStream::new(ws_stream, false))
        .map_err(|e| {
            println!("Error while accepting a connection from another node: {}", e);
            io::Error::new(io::ErrorKind::Other, e)
        })
    )
}

#[derive(Clone, Debug)]
pub struct SharedNimiqMessageStream {
    inner: MultiLock<NimiqMessageStream>,
    state: PublicStreamState,
}

impl SharedNimiqMessageStream {
    pub fn net_address(&self) -> NetAddress {
        self.state.net_address.clone()
    }

    pub fn outbound(&self) -> bool {
        self.state.outbound
    }

    pub fn last_chunk_received_at(&self) -> Option<&Instant> {
        self.state.last_chunk_received_at.as_ref()
    }

    #[cfg(feature = "metrics")]
    pub fn network_metrics(&self) -> &Arc<NetworkMetrics> {
        &self.state.network_metrics
    }
}

impl From<NimiqMessageStream> for SharedNimiqMessageStream {
    fn from(stream: NimiqMessageStream) -> Self {
        let state = PublicStreamState::from(&stream);
        SharedNimiqMessageStream {
            inner: MultiLock::new(stream),
            state,
        }
    }
}

impl Stream for SharedNimiqMessageStream {
    type Item = NimiqWebSocketMessage;
    type Error = NimiqMessageStreamError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.poll();
                self.state.update(&inner.public_state);
                result
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl Sink for SharedNimiqMessageStream {
    type SinkItem = NimiqWebSocketMessage;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem)
                  -> StartSend<Self::SinkItem, Self::SinkError>
    {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.start_send(item);
                self.state.update(&inner.public_state);
                result
            },
            Async::NotReady => Ok(AsyncSink::NotReady(item)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.poll_complete();
                self.state.update(&inner.public_state);
                result
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.close();
                self.state.update(&inner.public_state);
                result
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
