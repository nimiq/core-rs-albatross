use std::fmt;
use std::sync::Arc;

use futures::prelude::*;
use futures::sync::mpsc::*;
use parking_lot::Mutex;
use tokio::prelude::{Stream};

use crate::consensus::base::primitive::hash::Argon2dHash;
use crate::network::message::Message;
use crate::network::websocket::NimiqMessageStreamError;
use crate::network::websocket::SharedNimiqMessageStream;
use crate::network::peer::Peer;
use crate::network::connection::network_connection::AddressInfo;
use crate::utils::observer::Notifier;
use std::fmt::Debug;
use crate::network::connection::close_type::CloseType;
use crate::utils::observer::Listener;
use crate::network::connection::network_connection::NetworkConnection;
use parking_lot::RwLock;
use crate::utils::observer::ListenerHandle;

#[derive(Debug)]
pub enum ProtocolError {
    SendError(SendError<Message>),
}

pub trait Agent: Send {
    /// Initialize the protocol.
    fn initialize(&mut self) {}

    /// Maintain the protocol state.
//    fn maintain(&mut self) {}

    /// Handle a message.
    fn on_message(&mut self, msg: &Message) -> Result<(), ProtocolError>;

    /// On disconnect.
    fn on_close(&mut self) {}

    /// Boxes the protocol.
    fn boxed(self) -> Box<Agent> where Self: Sized + 'static {
        Box::new(self)
    }
}

#[derive(Debug)]
pub struct PingAgent {
    sink: PeerSink,
}

impl PingAgent {
    pub fn new(sink: PeerSink) -> Self {
        PingAgent {
            sink,
        }
    }
}

impl Agent for PingAgent {
    fn on_message(&mut self, msg: &Message) -> Result<(), ProtocolError> {
        if let Message::Ping(nonce) = msg {
            // Respond with a pong message.
            self.sink.send(Message::Pong(*nonce))
                .map_err(|err| ProtocolError::SendError(err))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub struct PeerChannel<'conn> {
    stream_notifier: Arc<RwLock<Notifier<'conn, PeerStreamEvent>>>,
    pub notifier: Arc<RwLock<Notifier<'conn, PeerChannelEvent>>>,
    network_connection_listener_handle: ListenerHandle,
    peer_sink: PeerSink,
    pub address_info: AddressInfo,
}

impl<'conn> PeerChannel<'conn> {
    pub fn new(network_connection: Arc<NetworkConnection<'conn>>, address_info: AddressInfo) -> PeerChannel {
        let notifier = Arc::new(RwLock::new(Notifier::new()));
        let bubble_notifier = notifier.clone();
        let network_connection_listener_handle = network_connection.notifier.write().register(move |e| {
            bubble_notifier.read().notify(PeerChannelEvent::from(e));
        });

        PeerChannel {
            stream_notifier: network_connection.notifier.clone(),
            notifier,
            network_connection_listener_handle,
            peer_sink: network_connection.peer_sink(),
            address_info,
        }
    }
}

impl<'conn> Drop for PeerChannel<'conn> {
    fn drop(&mut self) {
        self.stream_notifier.write().deregister(self.network_connection_listener_handle);
    }
}

impl<'conn> Debug for PeerChannel<'conn> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerChannel {{}}")
    }
}

#[derive(Clone)]
pub enum PeerChannelEvent {
    Message(Arc<Message>),
    Close(CloseType),
    Error, // cannot use `NimiqMessageStreamError`, because `tungstenite::Error` is not `Clone`
}

impl From<PeerStreamEvent> for PeerChannelEvent {
    fn from(e: PeerStreamEvent) -> Self {
        match e {
            PeerStreamEvent::Message(msg) => PeerChannelEvent::Message(msg),
            PeerStreamEvent::Close(ty) => PeerChannelEvent::Close(ty),
            PeerStreamEvent::Error => PeerChannelEvent::Error,
        }
    }
}

#[derive(Clone)]
pub struct PeerSink {
    sink: UnboundedSender<Message>,
}

impl PeerSink {
    pub fn new(channel: UnboundedSender<Message>) -> Self {
        PeerSink {
            sink: channel.clone(),
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<Message>> {
        self.sink.unbounded_send(msg)
    }
}

impl Debug for PeerSink {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerSink {{}}")
    }
}

#[derive(Clone)]
pub enum PeerStreamEvent {
    Message(Arc<Message>),
    Close(CloseType),
    Error, // cannot use `NimiqMessageStreamError`, because `tungstenite::Error` is not `Clone`
}

pub struct PeerStream<'conn> {
    stream: SharedNimiqMessageStream,
    notifier: Arc<RwLock<Notifier<'conn, PeerStreamEvent>>>,
}

impl<'conn> PeerStream<'conn> {
    pub fn new(stream: SharedNimiqMessageStream, notifier: Arc<RwLock<Notifier<'conn, PeerStreamEvent>>>) -> Self {
        PeerStream {
            stream,
            notifier,
        }
    }

    pub fn process_stream(self) -> impl Future<Item=(), Error=NimiqMessageStreamError> + 'conn {
        let stream = self.stream;
        let msg_notifier = self.notifier.clone();
        let error_notifier = self.notifier.clone();
        let close_notifier = self.notifier;

        let process_message = stream.for_each(move |msg| {
            msg_notifier.read().notify(PeerStreamEvent::Message(Arc::new(msg)));
            Ok(())
        }).or_else(move |error| {
            error_notifier.read().notify(PeerStreamEvent::Error);
            Err(error)
        }).and_then(move |result| {
            close_notifier.read().notify(PeerStreamEvent::Close(CloseType::ClosedByRemote));
            Ok(result)
        });

        process_message
    }
}

impl<'conn> Debug for PeerStream<'conn> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.stream.fmt(f)
    }
}
