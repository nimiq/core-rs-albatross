use std::collections::vec_deque::VecDeque;
use std::fmt;
use std::fmt::Debug;
#[cfg(feature = "metrics")]
use std::sync::Arc;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Sink, Stream};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tungstenite::error::Error as WebSocketError;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::Message as WebSocketMessage;

use beserial::{Deserialize, Serialize};
use network_primitives::address::net_address::NetAddress;
use network_messages::Message as NimiqMessage;

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

pub type NimiqMessageStream = MessageStream<WebSocketLayer>;

/// This struct encapsulates the transport layer
/// and instead sends/receives our own Message type encapsulating Nimiq messages.
// TODO Better name
// TODO Decouple from WebSocketMessage, make it some generic enum { Vec<u8>; CloseType }
pub struct MessageStream<S>
    where S: Stream + Sink<WebSocketMessage>
{
    // Internal state.
    inner: S,
    receiving_tag: u8,
    sending_tag: u8,
    recv_queue: VecDeque<WebSocketMessage>,
    recv_buf: Option<Vec<u8>>,
    send_queue: VecDeque<Vec<u8>>,
    state: WebSocketState,

    // Public state.
    pub(crate) public_state: PublicStreamInfo,
}

impl<S> MessageStream<S>
    where S: Stream + Sink<WebSocketMessage>
{
    pub(super) fn new(transport: S, net_address: NetAddress, outbound: bool) -> Self {
        Self {
            inner: transport,
            receiving_tag: 254,
            sending_tag: 0,
            recv_queue: VecDeque::new(),
            recv_buf: None,
            send_queue: VecDeque::new(),
            state: WebSocketState::Active,

            public_state: PublicStreamInfo::new(net_address, outbound),
        }
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

    fn is_ready(poll: &Poll<Result<(), Error>>) -> bool {
        match poll {
            &Poll::Ready(Ok(())) => true,
            _ => false,
        }
    }
}

impl Sink<Message> for NimiqMessageStream {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        loop {
            match Sink::poll_ready(Pin::new(&mut self.inner), cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) =>
                    return Poll::Ready(Err(Error::WebSocketError(err))),
                Poll::Ready(Ok(())) => (),
            };
            // Send all queued chunks before reporting to be ready.
            // TODO Allow pipelining?
            if let Some(buf) = self.send_queue.pop_back() {
                #[cfg(feature = "metrics")]
                    let buffer_len = buf.len();
                Sink::start_send(Pin::new(&mut self.inner), WebSocketMessage::binary(buf))
                    .map_err(Error::WebSocketError)?;
                #[cfg(feature = "metrics")]
                    self.public_state.network_metrics.note_bytes_sent(buffer_len);
            } else {
                return Poll::Ready(Ok(()));
            }
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        // Begin a new message
        let (serialized_msg, tag) = match item {
            // A message needs to be serialized and send with a new tag.
            Message::Message(msg) => {
                let serialized_msg = msg.serialize_to_vec();
                (serialized_msg, self.next_tag())
            },
            // Close frames need to be handled differently.
            Message::Close(frame) => {
                self.state = WebSocketState::ClosedByUs;
                return Sink::start_send(Pin::new(&mut self.inner), WebSocketMessage::Close(frame))
                    .map_err(Error::WebSocketError);
            },
        };
        if !self.send_queue.is_empty() {
            panic!("Previous message not flushed but trying to send another one (did you check poll_ready?)");
        }

        // Send chunks to underlying layer.
        let mut remaining = serialized_msg.len();
        while remaining > 0 {
            // Build next chunk
            let mut buffer;
            let chunk;
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
            self.send_queue.push_back(buffer);
            remaining -= chunk.len();
        }

        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let ready_poll = Sink::poll_ready(Pin::new(&mut *self), cx);
        if Self::is_ready(&ready_poll) {
            return ready_poll;
        }
        Sink::poll_flush(Pin::new(&mut self.inner), cx)
            .map(|res| res.map_err(Error::WebSocketError))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let ready_poll = Sink::poll_ready(Pin::new(&mut *self), cx);
        if Self::is_ready(&ready_poll) {
            return ready_poll;
        }
        // TODO Send close frame?
        Sink::poll_close(Pin::new(&mut self.inner), cx)
            .map(|res| res.map_err(Error::WebSocketError))
    }
}

impl NimiqMessageStream {
    fn try_read_message(&mut self) -> Result<Option<Message>, Error> {
        // If there are no web socket messages in the buffer, signal that we don't have anything yet
        // (i.e. we would need to block waiting, which is a no no in an async function)
        if self.recv_queue.is_empty() {
            return Ok(None);
        }

        while let Some(ws_msg) = self.recv_queue.pop_front() {
            let raw_msg = ws_msg.into_data();
            // We need at least the tag.
            if raw_msg.is_empty() {
                return Err(Error::InvalidMessageFormat);
            }

            let tag = raw_msg[0];
            let chunk = &raw_msg[1..];

            // Detect if this is a new message.
            if self.recv_buf.is_none() {

                if let Ok(msg_size) = NimiqMessage::peek_length(chunk) {
                    if msg_size > MAX_MESSAGE_SIZE {
                        error!("Max message size exceeded ({} > {})", msg_size, MAX_MESSAGE_SIZE);
                        return Err(Error::MessageSizeExceeded);
                    }
                    self.recv_buf = Some(Vec::with_capacity(msg_size));
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

            let msg_buf = self.recv_buf.as_mut().unwrap();
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
                self.recv_buf = None;

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
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.state.is_closed() {
            return Poll::Ready(None);
        }

        // First, lets get as many WebSocket messages as available and store them in the buffer.
        loop {
            match Stream::poll_next(Pin::new(&mut self.inner), cx) {
                // Handle close frames first.
                Poll::Ready(Some(Ok(WebSocketMessage::Close(frame)))) => {
                    // If we haven't closed the connection, note as closed by peer.
                    if !self.state.is_closed() {
                        self.state = WebSocketState::ClosedByPeer(frame.clone());
                    }

                    return Poll::Ready(Some(Ok(Message::Close(frame))))
                },
                Poll::Ready(Some(Ok(m))) => {
                    #[cfg(feature = "metrics")]
                        self.public_state.network_metrics.note_bytes_received(m.len());

                    // Check max chunk size.
                    if m.len() > MAX_CHUNK_SIZE {
                        error!("Max chunk size exceeded ({} > {})", m.len(), MAX_CHUNK_SIZE);
                        return Poll::Ready(Some(Err(Error::ChunkSizeExceeded)));
                        // TODO Behavior of subsequent polls
                    }
                    self.recv_queue.push_back(m);

                    if let Some(msg) = self.try_read_message()? {
                        return Poll::Ready(Some(Ok(msg)));
                    }
                },
                Poll::Ready(Some(Err(e))) => {
                    if let WebSocketError::ConnectionClosed = e {
                        // If we haven't closed the connection, note as closed by peer.
                        if !self.state.is_closed() {
                            self.state = WebSocketState::ClosedByPeer(None);
                        }
                    }
                    return Poll::Ready(Some(Err(e.into())))
                    // TODO Behavior of subsequent polls
                },
                Poll::Ready(None) => {
                    // Try to read a message from our buffer, but return early if it fails.
                    return match self.try_read_message()? {
                        Some(msg) => Poll::Ready(Some(Ok(msg))),
                        None => Poll::Ready(None),
                    };
                },
                Poll::Pending => {
                    break
                },
            }
        }

        Poll::Pending
    }
}

impl Debug for NimiqMessageStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "NimiqMessageStream {{}}")
    }
}
