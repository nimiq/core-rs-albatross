use std::borrow::Cow;
use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use futures::prelude::*;
use futures::sync::mpsc::*;
use parking_lot::RwLock;
use tokio::prelude::Stream;
use tungstenite::error::Error as WsError;
use tungstenite::protocol::CloseFrame;

use network_messages::{Message, MessageNotifier};
use utils::observer::{Notifier, PassThroughNotifier};
use utils::unique_id::UniqueId;
use utils::unique_ptr::UniquePtr;

use crate::connection::close_type::CloseType;
use crate::connection::network_connection::AddressInfo;
use crate::connection::network_connection::ClosedFlag;
use crate::connection::network_connection::NetworkConnection;
#[cfg(feature = "metrics")]
use crate::network_metrics::MessageMetrics;
use crate::websocket::{NimiqMessageStreamError, NimiqWebSocketMessage, SharedNimiqMessageStream};

#[derive(Clone)]
pub struct PeerChannel {
    stream_notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
    pub msg_notifier: Arc<MessageNotifier>,
    pub close_notifier: Arc<RwLock<Notifier<'static, CloseType>>>,
    peer_sink: PeerSink,
    pub address_info: AddressInfo,
    closed_flag: ClosedFlag,

    #[cfg(feature = "metrics")]
    pub message_metrics: Arc<MessageMetrics>,
}

impl PeerChannel {
    pub fn new(network_connection: &NetworkConnection) -> Self {
        let msg_notifier = Arc::new(MessageNotifier::new());
        let close_notifier = Arc::new(RwLock::new(Notifier::new()));

        #[cfg(feature = "metrics")]
        let message_metrics = Arc::new(MessageMetrics::new());

        let msg_notifier1 = msg_notifier.clone();
        let close_notifier1 = close_notifier.clone();

        #[cfg(feature = "metrics")]
        let message_metrics1 = message_metrics.clone();

        network_connection.notifier.write().register(move |e: PeerStreamEvent| {
            match e {
                PeerStreamEvent::Message(msg) => {
                    #[cfg(feature = "metrics")]
                    message_metrics1.note_message(msg.ty());
                    msg_notifier1.notify(msg)
                },
                PeerStreamEvent::Close(ty) => close_notifier1.read().notify(ty),
                PeerStreamEvent::Error(_) => {}
            }
        });

        PeerChannel {
            stream_notifier: network_connection.notifier.clone(),
            msg_notifier,
            close_notifier,
            peer_sink: network_connection.peer_sink(),
            address_info: network_connection.address_info(),
            closed_flag: network_connection.closed_flag(),

            #[cfg(feature = "metrics")]
            message_metrics,
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<NimiqWebSocketMessage>> {
        self.peer_sink.send(msg)
    }

    pub fn send_or_close(&self, msg: Message) {
        if self.peer_sink.send(msg).is_err() {
            let _ = self.peer_sink.close(CloseType::SendFailed, Some("SendFailed".to_string())); // TODO: We ignore the error here.
        }
    }

    pub fn closed(&self) -> bool {
        self.closed_flag.is_closed()
    }

    pub fn close(&self, ty: CloseType) -> bool {
        self.peer_sink.close(ty, None).is_ok()
    }
}

impl Debug for PeerChannel {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerChannel {{}}")
    }
}

impl PartialEq for PeerChannel {
    fn eq(&self, other: &PeerChannel) -> bool {
        self.peer_sink == other.peer_sink
    }

    fn ne(&self, other: &PeerChannel) -> bool {
        self.peer_sink != other.peer_sink
    }
}

impl Eq for PeerChannel {}

impl Hash for PeerChannel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.peer_sink.hash(state);
    }
}

pub enum PeerChannelEvent {
    Message(Message),
    Close(CloseType),
    Error(UniquePtr<NimiqMessageStreamError>),
}

impl From<PeerStreamEvent> for PeerChannelEvent {
    fn from(e: PeerStreamEvent) -> Self {
        match e {
            PeerStreamEvent::Message(msg) => PeerChannelEvent::Message(msg),
            PeerStreamEvent::Close(ty) => PeerChannelEvent::Close(ty),
            PeerStreamEvent::Error(error) => PeerChannelEvent::Error(error),
        }
    }
}

#[derive(Clone)]
pub struct PeerSink {
    sink: UnboundedSender<NimiqWebSocketMessage>,
    unique_id: UniqueId,
}

impl PeerSink {
    pub fn new(channel: UnboundedSender<NimiqWebSocketMessage>, unique_id: UniqueId) -> Self {
        PeerSink {
            sink: channel,
            unique_id,
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<NimiqWebSocketMessage>> {
        self.sink.unbounded_send(NimiqWebSocketMessage::Message(msg))
    }

    /// Closes the connection.
    pub fn close(&self, ty: CloseType, reason: Option<String>) -> Result<(), SendError<NimiqWebSocketMessage>> {
        self.sink.unbounded_send(NimiqWebSocketMessage::Close(Some(CloseFrame {
            code: ty.into(),
            reason: Cow::Owned(reason.unwrap_or_else(|| "Unknown reason".to_string())),
        })))
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

pub enum PeerStreamEvent {
    Message(Message),
    Close(CloseType),
    Error(UniquePtr<NimiqMessageStreamError>),
}

pub struct PeerStream {
    stream: SharedNimiqMessageStream,
    closed_flag: ClosedFlag,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
}

impl PeerStream {
    pub fn new(stream: SharedNimiqMessageStream, notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>, closed_flag: ClosedFlag) -> Self {
        PeerStream {
            stream,
            notifier,
            closed_flag,
        }
    }

    pub fn process_stream(self) -> impl Future<Item=(), Error=NimiqMessageStreamError> + 'static {
        let stream = self.stream;
        let msg_notifier = self.notifier.clone();
        let error_notifier = self.notifier;
        let msg_closed_flag = self.closed_flag.clone();
        let error_closed_flag = self.closed_flag;

        let process_message = stream.for_each(move |msg| {
            match msg {
                NimiqWebSocketMessage::Message(msg) => {
                    msg_notifier.read().notify(PeerStreamEvent::Message(msg));
                },
                NimiqWebSocketMessage::Close(frame) => {
                    msg_closed_flag.update(true);
                    msg_notifier.read().notify(PeerStreamEvent::Close(frame.map(|f| f.code).into()));
                },
            }
            Ok(())
        }).or_else(move |error| {
            match &error {
                NimiqMessageStreamError::WebSocketError(WsError::ConnectionClosed(ref frame)) => {
                    error_closed_flag.update(true);
                    error_notifier.read().notify(PeerStreamEvent::Close(frame.as_ref().map(|f| f.code).into()));
                },
                error => {
                    error_notifier.read().notify(PeerStreamEvent::Error(UniquePtr::new(error)));
                },
            }
            Err(error)
        });

        process_message
    }
}

impl Debug for PeerStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.stream.fmt(f)
    }
}
