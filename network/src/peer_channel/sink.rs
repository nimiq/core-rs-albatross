use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::hash::Hasher;

use futures::channel::mpsc::*;

use network_messages::Message;
use utils::unique_id::UniqueId;

use crate::connection::close_type::CloseType;
use crate::websocket::Message as WebSocketMessage;
use crate::connection::network_connection::ClosedFlag;

#[derive(Clone)]
pub struct PeerSink {
    sink: UnboundedSender<WebSocketMessage>,
    unique_id: UniqueId,
    closed_flag: ClosedFlag,
}

impl PeerSink {
    pub fn new(channel: UnboundedSender<WebSocketMessage>, unique_id: UniqueId, closed_flag: ClosedFlag) -> Self {
        PeerSink {
            sink: channel,
            unique_id,
            closed_flag,
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), TrySendError<WebSocketMessage>> {
        // Do not send messages over already closed connections.
        // Stop sending silently until connection is really closed.
        if self.closed_flag.is_closed() {
            return Ok(());
        }
        self.sink.unbounded_send(WebSocketMessage::Message(msg))
    }

    /// Closes the connection.
    pub fn close(&self, ty: CloseType, reason: Option<String>) {
        // Immediately mark channel as closed, so that no more messages are sent over it.
        // Do not send messages over already closed connections.
        // Stop sending silently until connection is really closed.
        if self.closed_flag.set_closed(true) {
            return;
        }
        self.closed_flag.set_close_type(ty);
        debug!("Closing connection, reason: {:?} ({:?})", ty, reason);
        if let Err(error) = self.sink.unbounded_send(WebSocketMessage::Close(None)) {
            debug!("Error closing connection: {}", error);
        }

        /*
         * Theoretically, we can also send the CloseType to the other party:
         * Some(CloseFrame {
         *     code: ty.into(),
         *     reason: Cow::Owned(reason.unwrap_or_else(|| "Unknown reason".to_string())),
         * })
         */
    }
}

impl Debug for PeerSink {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerSink {{}}")
    }
}

impl PartialEq for PeerSink {
    fn eq(&self, other: &PeerSink) -> bool {
        self.unique_id == other.unique_id
    }
}

impl Eq for PeerSink {}

impl Hash for PeerSink {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.unique_id.hash(state);
    }
}
