use std::collections::vec_deque::VecDeque;
use std::fmt;
use std::fmt::Debug;
#[cfg(feature = "metrics")]
use std::sync::Arc;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Sink, Stream};
use futures::stream::{SplitStream, SplitSink};
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

pub type NimiqMessageStream = MessageStream<SplitStream<WebSocketLayer>, SplitSink<WebSocketLayer, WebSocketMessage>>;

/// This struct encapsulates the transport layer
/// and instead sends/receives our own Message type encapsulating Nimiq messages.
// TODO Better name
// TODO Decouple from WebSocketMessage, make it some generic enum { Vec<u8>; CloseType }
// TODO Split stream/sink functionality, use futures::stream::Merge
// TODO Split chunker from message (de-)serializer
pub struct MessageStream<R, T> {
    // Internal state.
    rx: R,
    rx_tag: u8,
    rx_queue: VecDeque<WebSocketMessage>,
    rx_buf: Option<Vec<u8>>,

    tx: T,
    tx_tag: u8,
    tx_queue: VecDeque<Vec<u8>>,
    state: WebSocketState,

    // Public state.
    pub(crate) public_state: PublicStreamInfo,
}

impl<R, T> MessageStream<R, T>
    where R: Stream,
          T: Sink<WebSocketMessage, Error=WebSocketError>
{
    pub(super) fn new(rx: R, tx: T, net_address: NetAddress, outbound: bool) -> Self {
        Self {
            rx,
            rx_tag: 254,
            rx_queue: VecDeque::new(),
            tx_queue: VecDeque::new(),

            tx,
            tx_tag: 0,
            rx_buf: None,
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
        let tag = self.tx_tag;
        // XXX JS implementation quirk: Already wrap at 255 instead of 256
        self.tx_tag = (self.tx_tag + 1) % 255;
        tag
    }

    #[cfg(feature = "metrics")]
    pub fn network_metrics(&self) -> &Arc<NetworkMetrics> {
        &self.public_state.network_metrics
    }

    fn is_ready(poll: &Poll<Result<(), Error>>) -> bool {
        match *poll {
            Poll::Ready(Ok(())) => true,
            _ => false,
        }
    }
}

impl<R, T> Sink<Message> for MessageStream<R, T>
    where R: Stream + Unpin,
          T: Sink<WebSocketMessage, Error=WebSocketError> + Unpin
{
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        loop {
            match Sink::poll_ready(Pin::new(&mut self.tx), cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) =>
                    return Poll::Ready(Err(Error::WebSocketError(err))),
                Poll::Ready(Ok(())) => (),
            };
            // Send all queued chunks before reporting to be ready.
            if let Some(buf) = self.tx_queue.pop_front() {
                #[cfg(feature = "metrics")]
                    let buffer_len = buf.len();
                Sink::start_send(Pin::new(&mut self.tx), WebSocketMessage::binary(buf))
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
        let (serialized_msg, mut tag) = match item {
            // A message needs to be serialized and send with a new tag.
            Message::Message(msg) => {
                let serialized_msg = msg.serialize_to_vec();
                (serialized_msg, self.next_tag())
            }
            // Close frames need to be handled differently.
            Message::Close(frame) => {
                self.state = WebSocketState::ClosedByUs;
                return Sink::start_send(Pin::new(&mut self.tx), WebSocketMessage::Close(frame))
                    .map_err(Error::WebSocketError);
            }
        };
        if !self.tx_queue.is_empty() {
            panic!("Previous message not flushed but trying to send another one (did you check poll_ready?)");
        }

        // Send chunks to underlying layer.
        let mut remaining = serialized_msg.len();
        while remaining > 0 {
            // Build next chunk
            let mut buffer = Vec::with_capacity(MAX_CHUNK_SIZE);
            buffer.push(tag);
            let chunk;
            let start = serialized_msg.len() - remaining;
            if remaining >= MAX_CHUNK_SIZE - 1 {
                chunk = &serialized_msg[start..(start + MAX_CHUNK_SIZE - 1)];
                tag = self.next_tag();
            } else {
                chunk = &serialized_msg[start..];
            }
            buffer.extend(chunk);
            self.tx_queue.push_back(buffer);
            remaining -= chunk.len();
        }

        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let ready_poll = Sink::poll_ready(Pin::new(&mut *self), cx);
        if Self::is_ready(&ready_poll) {
            return ready_poll;
        }
        Sink::poll_flush(Pin::new(&mut self.tx), cx)
            .map(|res| res.map_err(Error::WebSocketError))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let ready_poll = Sink::poll_ready(Pin::new(&mut *self), cx);
        if Self::is_ready(&ready_poll) {
            return ready_poll;
        }
        // TODO Send close frame?
        Sink::poll_close(Pin::new(&mut self.tx), cx)
            .map(|res| res.map_err(Error::WebSocketError))
    }
}

impl<R, T> MessageStream<R, T>
    where R: Stream,
          T: Sink<WebSocketMessage>
{
    fn try_read_message(&mut self) -> Result<Option<Message>, Error> {
        // If there are no web socket messages in the buffer, signal that we don't have anything yet
        // (i.e. we would need to block waiting, which is a no no in an async function)
        if self.rx_queue.is_empty() {
            return Ok(None);
        }

        while let Some(ws_msg) = self.rx_queue.pop_front() {
            let raw_msg = ws_msg.into_data();
            // We need at least the tag.
            if raw_msg.is_empty() {
                return Err(Error::InvalidMessageFormat);
            }

            let tag = raw_msg[0];
            let chunk = &raw_msg[1..];

            // Detect if this is a new message.
            if self.rx_buf.is_none() {
                if let Ok(msg_size) = NimiqMessage::peek_length(chunk) {
                    if msg_size > MAX_MESSAGE_SIZE {
                        error!("Max message size exceeded ({} > {})", msg_size, MAX_MESSAGE_SIZE);
                        return Err(Error::MessageSizeExceeded);
                    }
                    self.rx_buf = Some(Vec::with_capacity(msg_size));
                } else {
                    return Err(Error::InvalidMessageFormat);
                }

                // XXX JS implementation quirk: Already wrap at 255 instead of 256
                self.rx_tag = (self.rx_tag + 1) % 255;
            }

            if self.rx_tag != tag {
                error!("Tag mismatch: expected {}, got {}", self.rx_tag, tag);
                return Err(Error::TagMismatch);
            }

            let msg_buf = self.rx_buf.as_mut().unwrap();
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
                self.rx_buf = None;

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

impl<R, T> Stream for MessageStream<R, T>
    where R: Stream<Item=Result<WebSocketMessage, WebSocketError>> + Unpin,
          T: Sink<WebSocketMessage> + Unpin
{
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.state.is_closed() {
            return Poll::Ready(None);
        }

        // First, lets get as many WebSocket messages as available and store them in the buffer.
        loop {
            match Stream::poll_next(Pin::new(&mut self.rx), cx) {
                // Handle close frames first.
                Poll::Ready(Some(Ok(WebSocketMessage::Close(frame)))) => {
                    // If we haven't closed the connection, note as closed by peer.
                    if !self.state.is_closed() {
                        self.state = WebSocketState::ClosedByPeer(frame.clone());
                    }

                    return Poll::Ready(Some(Ok(Message::Close(frame))));
                }
                Poll::Ready(Some(Ok(m))) => {
                    #[cfg(feature = "metrics")]
                        self.public_state.network_metrics.note_bytes_received(m.len());

                    // Check max chunk size.
                    if m.len() > MAX_CHUNK_SIZE {
                        error!("Max chunk size exceeded ({} > {})", m.len(), MAX_CHUNK_SIZE);
                        return Poll::Ready(Some(Err(Error::ChunkSizeExceeded)));
                        // TODO Behavior of subsequent polls
                    }
                    self.rx_queue.push_back(m);

                    if let Some(msg) = self.try_read_message()? {
                        return Poll::Ready(Some(Ok(msg)));
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    if let WebSocketError::ConnectionClosed = e {
                        // If we haven't closed the connection, note as closed by peer.
                        if !self.state.is_closed() {
                            self.state = WebSocketState::ClosedByPeer(None);
                        }
                    }
                    return Poll::Ready(Some(Err(e.into())));
                    // TODO Behavior of subsequent polls
                }
                Poll::Ready(None) => {
                    // Try to read a message from our buffer, but return early if it fails.
                    return match self.try_read_message()? {
                        Some(msg) => Poll::Ready(Some(Ok(msg))),
                        None => Poll::Ready(None),
                    };
                }
                Poll::Pending => {
                    break;
                }
            }
        }

        Poll::Pending
    }
}

impl<R, T> Debug for MessageStream<R, T>
    where R: Stream,
          T: Sink<WebSocketMessage>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        // TODO Include rx/tx tags
        write!(f, "MessageStream {{}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::SinkExt;

    use nimiq_messages::{MessageType, RejectMessage, RejectMessageCode};

    // send_messages writes Nimiq messages to a mock transport and returns the would-be WS messages
    async fn send_messages<I>(nimiq_messages: I) -> Vec<WebSocketMessage>
        where I: Iterator<Item=Message>
    {
        // Mock transport
        let mut target = Vec::new();
        let target_ref = &mut target;
        let tx = target_ref.sink_map_err(|_| WebSocketError::AlreadyClosed);
        let rx = futures::stream::empty::<Message>();
        let mut stream = MessageStream::new(rx, tx, NetAddress::Unknown, false);
        // Write all messages
        for msg in nimiq_messages {
            assert!(stream.send(msg).await.is_ok());
        }
        target
    }

    #[tokio::test]
    async fn stream_can_send_small_messages() {
        let ws = send_messages(vec![
            Message::Message(NimiqMessage::Ping(42)),
            Message::Message(NimiqMessage::Ping(13)),
        ].into_iter()).await;
        assert_eq!(ws_hex(&ws[0]), "00420420421600000011c854a3280000002a");
        assert_eq!(ws_hex(&ws[1]), "014204204216000000116d5e16430000000d");
    }

    #[tokio::test]
    async fn stream_can_send_large_messages() {
        let blob = vec![0xFFu8; 2 * MAX_CHUNK_SIZE];
        let proto_msg = RejectMessage::new(
            MessageType::Version,
            RejectMessageCode::Malformed,
            "".to_string(),
            Some(blob)
        );
        let ws = send_messages(vec![
            // 1 chunk
            Message::Message(NimiqMessage::Ping(0xCC)),
            // 3 chunks
            Message::Message(proto_msg),
            // 1 chunk
            Message::Message(NimiqMessage::Ping(0xCC)),
        ].into_iter()).await;
        assert_eq!(ws.len(), 5);
        // Check small messages
        assert_eq!(ws_hex(&ws[0]), "00420420421600000011813de465000000cc");
        assert_eq!(ws_hex(&ws[4]), "04420420421600000011813de465000000cc");
        // Check large message sizes
        assert_eq!(ws_data(&ws[1]).len(), MAX_CHUNK_SIZE);
        assert_eq!(ws_data(&ws[2]).len(), MAX_CHUNK_SIZE);
        assert_eq!(ws_data(&ws[3]).len(), 21);
        // Check chunk prefixes
        assert_eq!(&hex::encode(&ws_data(&ws[1])[..19]), "01420420420a00008012175d4ff90001008000");
        assert_eq!(&hex::encode(&ws_data(&ws[2])[..1]), "02");
        assert_eq!(&hex::encode(&ws_data(&ws[3])[..1]), "03");
        // Check rest
        ws_data(&ws[1])[19..].iter().for_each(|b| assert_eq!(*b, 0xFF));
        ws_data(&ws[2])[1..].iter().for_each(|b| assert_eq!(*b, 0xFF));
        ws_data(&ws[3])[1..].iter().for_each(|b| assert_eq!(*b, 0xFF));
    }

    #[tokio::test]
    async fn stream_sends_correct_tags() {
        let messages = std::iter::repeat(Message::Message(NimiqMessage::Ping(42))).take(257);
        let ws = send_messages(messages).await;
        assert_eq!(ws.len(), 257);
        for i in 0..0xFF {
            assert_eq!(ws[i].clone().into_data()[0], i as u8);
        }
        assert_eq!(ws_data(&ws[0xFF])[0], 0x00);
        assert_eq!(ws_data(&ws[0x100])[0], 0x01);
    }

    fn ws_data(ws: &WebSocketMessage) -> &[u8] {
        match ws {
            WebSocketMessage::Binary(vec) => vec,
            _ => panic!("Not a binary frame!"),
        }
    }

    fn ws_hex(ws: &WebSocketMessage) -> String {
        hex::encode(ws_data(ws))
    }
}
