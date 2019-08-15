use std::collections::vec_deque::VecDeque;
use std::fmt;
use std::fmt::Debug;
use std::net;
#[cfg(feature = "metrics")]
use std::sync::Arc;

use futures::prelude::*;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::stream::PeerAddr;
use tungstenite::error::Error as WebSocketError;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::Message as WebSocketMessage;

use beserial::{Deserialize, Serialize};
use network_messages::Message as NimiqMessage;
use network_primitives::address::net_address::NetAddress;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::websocket::error::Error;
use crate::websocket::Message;
use crate::websocket::public_state::PublicStreamInfo;

type WebSocketLayer = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// This enum describes the current state of the connection.
#[derive(Clone, Debug)]
pub enum WebSocketState {
    /// The connection is active.
    Active,
    /// We initiated a close handshake.
    ClosedByUs,
    /// The peer initiated a close handshake.
    ClosedByPeer(Option<CloseFrame<'static>>),
}

impl WebSocketState {
    #[inline]
    pub fn is_active(&self) -> bool {
        match self {
            WebSocketState::Active => true,
            _ => false,
        }
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        !self.is_active()
    }
}

const MAX_CHUNK_SIZE: usize = 1024 * 16; // 16 kb
const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 10; // 10 mb

/// This struct encapsulates the underlying WebSocket layer
/// and instead sends/receives our own Message type encapsulating Nimiq messages.
pub struct NimiqMessageStream {
    // Internal state.
    inner: WebSocketLayer,
    receiving_tag: u8,
    sending_tag: u8,
    ws_queue: VecDeque<WebSocketMessage>,
    msg_buf: Option<Vec<u8>>,
    state: WebSocketState,

    // Public state.
    pub(crate) public_state: PublicStreamInfo,
}

impl NimiqMessageStream {
    pub(super) fn new(ws_socket: WebSocketLayer, outbound: bool) -> Result<Self, Error> {
        let peer_addr = ws_socket.peer_addr().map_err(Error::NetAddressMissing)?;
        Ok(NimiqMessageStream {
            inner: ws_socket,
            receiving_tag: 254,
            sending_tag: 0,
            ws_queue: VecDeque::new(),
            msg_buf: None,
            state: WebSocketState::Active,

            public_state: PublicStreamInfo::new(match peer_addr.ip() {
                net::IpAddr::V4(ip4) => NetAddress::IPv4(ip4),
                net::IpAddr::V6(ip6) => NetAddress::IPv6(ip6),
            }, outbound),
        })
    }

    pub fn state(&self) -> &PublicStreamInfo {
        &self.public_state
    }

    pub fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    fn next_tag(&mut self) -> u8 {
        // Save and increment tag.
        let tag = self.sending_tag;
        // XXX JS implementation quirk: Already wrap at 255 instead of 256
        self.sending_tag = (self.sending_tag + 1) % 255;
        tag
    }

    #[cfg(feature = "metrics")]
    pub fn network_metrics(&self) -> &Arc<NetworkMetrics> {
        &self.public_state.network_metrics
    }
}

impl Sink for NimiqMessageStream {
    type SinkItem = Message;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        let (serialized_msg, tag) = match item {
            // A message needs to be serialized and send with a new tag.
            Message::Message(msg) => {
                let serialized_msg = msg.serialize_to_vec();
                (serialized_msg, self.next_tag())
            },
            // If sending of a message was interrupted due to a full queue
            // we resume sending with the previous tag (if given).
            Message::Resume(serialized_msg, sending_tag) => {
                let tag = match sending_tag {
                    Some(tag) => tag,
                    None => self.next_tag(),
                };
                (serialized_msg, tag)
            },
            // Close frames need to be handled differently.
            Message::Close(frame) => {
                self.state = WebSocketState::ClosedByUs;

                return match self.inner.start_send(WebSocketMessage::Close(frame)) {
                    Ok(state) => match state {
                        AsyncSink::Ready => Ok(AsyncSink::Ready),
                        AsyncSink::NotReady(WebSocketMessage::Close(frame)) => Ok(AsyncSink::NotReady(Message::Close(frame))),
                        AsyncSink::NotReady(_) => {
                            error!("Expected to get NotReady of a Close message, but got something else.");
                            Err(Error::InvalidClosingState)
                        },
                    },
                    Err(err) => Err(Error::WebSocketError(err)),
                }
            },
        };

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

            #[cfg(feature = "metrics")]
            let buffer_len = buffer.len();
            match self.inner.start_send(WebSocketMessage::binary(buffer)) {
                Ok(state) => match state {
                    AsyncSink::Ready => {
                        #[cfg(feature = "metrics")]
                        self.public_state.network_metrics.note_bytes_sent(buffer_len);
                    },
                    // We started to send some chunks, but now the queue is full:
                    // If this happens, we will try sending the rest of the message at a later point with the same tag.
                    AsyncSink::NotReady(_) => return Ok(AsyncSink::NotReady(Message::Resume(serialized_msg[start..].to_vec(), Some(tag)))),
                },
                Err(error) => return Err(Error::WebSocketError(error)),
            };

            remaining -= chunk.len();
        }
        // We didn't exit previously, so everything worked out.
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_complete() {
            Ok(r_async) => Ok(r_async),
            Err(error) => Err(Error::WebSocketError(error)),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.close() {
            Ok(r_async) => Ok(r_async),
            Err(error) => Err(Error::WebSocketError(error)),
        }
    }
}

impl NimiqMessageStream {
    fn try_read_message(&mut self) -> Result<Option<Message>, Error> {
        // If there are no web socket messages in the buffer, signal that we don't have anything yet
        // (i.e. we would need to block waiting, which is a no no in an async function)
        if self.ws_queue.is_empty() {
            return Ok(None);
        }

        while let Some(ws_msg) = self.ws_queue.pop_front() {
            let raw_msg = ws_msg.into_data();
            // We need at least the tag.
            if raw_msg.is_empty() {
                return Err(Error::InvalidMessageFormat);
            }

            let tag = raw_msg[0];
            let chunk = &raw_msg[1..];

            // Detect if this is a new message.
            if self.msg_buf.is_none() {

                if let Ok(msg_size) = NimiqMessage::peek_length(chunk) {
                    if msg_size > MAX_MESSAGE_SIZE {
                        error!("Max message size exceeded ({} > {})", msg_size, MAX_MESSAGE_SIZE);
                        return Err(Error::MessageSizeExceeded);
                    }
                    self.msg_buf = Some(Vec::with_capacity(msg_size));
                } else {
                    return Err(Error::InvalidMessageFormat);
                }

                // XXX JS implementation quirk: Already wrap at 255 instead of 256
                self.receiving_tag = (self.receiving_tag + 1) % 255;
            }

            if self.receiving_tag != tag {
                error!("Tag mismatch: expected {}, got {}", self.receiving_tag, tag);
                return Err(Error::TagMismatch);
            }

            let msg_buf = self.msg_buf.as_mut().unwrap();
            let mut remaining = msg_buf.capacity() - msg_buf.len();

            let chunk_size = raw_msg.len() - 1;
            if chunk_size > remaining {
                error!("Final chunk size exceeded ({} > {})", chunk_size, remaining);
                return Err(Error::FinalChunkSizeExceeded);
            }

            msg_buf.extend_from_slice(chunk);
            remaining -= chunk_size;

            if remaining == 0 {
                // Full message read, parse it.
                let msg = Deserialize::deserialize(&mut &msg_buf[..]);

                // Reset message buffer.
                self.msg_buf = None;

                match msg {
                    Err(e) => {
                        return Err(Error::ParseError(e));
                    }
                    Ok(msg) => {
                        return Ok(Some(Message::Message(msg)));
                    }
                }
            }
        }
        Ok(None)
    }
}

impl Stream for NimiqMessageStream {
    type Item = Message;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.state.is_closed() {
            return Ok(Async::Ready(None));
        }

        // First, lets get as many WebSocket messages as available and store them in the buffer.
        loop {
            match self.inner.poll() {
                // Handle close frames first.
                Ok(Async::Ready(Some(WebSocketMessage::Close(frame)))) => {
                    // If we haven't closed the connection, note as closed by peer.
                    if !self.state.is_closed() {
                        self.state = WebSocketState::ClosedByPeer(frame.clone());
                    }

                    return Ok(Async::Ready(Some(Message::Close(frame))))
                },
                Ok(Async::Ready(Some(m))) => {
                    #[cfg(feature = "metrics")]
                    self.public_state.network_metrics.note_bytes_received(m.len());

                    // Check max chunk size.
                    if m.len() > MAX_CHUNK_SIZE {
                        error!("Max chunk size exceeded ({} > {})", m.len(), MAX_CHUNK_SIZE);
                        return Err(Error::ChunkSizeExceeded);
                    }
                    self.ws_queue.push_back(m);

                    if let Some(msg) = self.try_read_message()? {
                        return Ok(Async::Ready(Some(msg)));
                    }
                },
                Ok(Async::Ready(None)) => {
                    // Try to read a message from our buffer, but return early if it fails.
                    return match self.try_read_message()? {
                        Some(msg) => Ok(Async::Ready(Some(msg))),
                        None => Ok(Async::Ready(None)),
                    };
                },
                Ok(Async::NotReady) => {
                    break
                },
                Err(e) => {
                    if let WebSocketError::ConnectionClosed = e {
                        // If we haven't closed the connection, note as closed by peer.
                        if !self.state.is_closed() {
                            self.state = WebSocketState::ClosedByPeer(None);
                        }
                    }
                    return Err(e.into())
                }
            }
        }

        Ok(Async::NotReady)
    }
}

impl Debug for NimiqMessageStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "NimiqMessageStream {{}}")
    }
}
