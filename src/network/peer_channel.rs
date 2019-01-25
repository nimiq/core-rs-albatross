use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;

use futures::prelude::*;
use futures::sync::mpsc::*;
use parking_lot::RwLock;
use tokio::prelude::Stream;

use crate::network::connection::close_type::CloseType;
use crate::network::connection::network_connection::AddressInfo;
use crate::network::connection::network_connection::NetworkConnection;
use crate::network::message::Message;
use crate::network::websocket::NimiqMessageStreamError;
use crate::network::websocket::SharedNimiqMessageStream;
use crate::utils::observer::{Notifier, PassThroughNotifier};
use crate::network::connection::network_connection::ClosingHelper;
use crate::utils::unique_ptr::UniquePtr;
use crate::network::message::MessageNotifier;
use std::hash::Hash;
use std::hash::Hasher;

#[derive(Clone)]
pub struct PeerChannel {
    stream_notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
    pub msg_notifier: Arc<MessageNotifier>,
    pub close_notifier: Arc<RwLock<Notifier<'static, CloseType>>>,
    peer_sink: PeerSink,
    pub address_info: AddressInfo,
}

impl PeerChannel {
    pub fn new(network_connection: &NetworkConnection) -> Self {
        let msg_notifier = Arc::new(MessageNotifier::new());
        let close_notifier = Arc::new(RwLock::new(Notifier::new()));

        let msg_notifier1 = msg_notifier.clone();
        let close_notifier1 = close_notifier.clone();
        network_connection.notifier.write().register(move |e: PeerStreamEvent| {
            match e {
                PeerStreamEvent::Message(msg) => msg_notifier1.notify(msg),
                PeerStreamEvent::Close(ty) => close_notifier1.read().notify(ty),
                PeerStreamEvent::Error(e) => {
                    // TODO close channel
                    error!("Got peer stream error: {:?}", *e);
                }
            }
        });

        PeerChannel {
            stream_notifier: network_connection.notifier.clone(),
            msg_notifier,
            close_notifier,
            peer_sink: network_connection.peer_sink(),
            address_info: network_connection.address_info(),
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<Message>> {
        self.peer_sink.send(msg)
    }

    pub fn send_or_close(&self, msg: Message) {
        if self.peer_sink.send(msg).is_err() {
            self.peer_sink.close(CloseType::SendFailed);
        }
    }

    pub fn closed(&self) -> bool {
        self.peer_sink.closed()
    }

    pub fn close(&self, ty: CloseType) -> bool {
        self.peer_sink.close(ty)
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
    sink: UnboundedSender<Message>,
    closing_helper: Arc<ClosingHelper>,
}

impl PeerSink {
    pub fn new(channel: UnboundedSender<Message>, closing_helper: ClosingHelper) -> Self {
        PeerSink {
            sink: channel,
            closing_helper: Arc::new(closing_helper),
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<Message>> {
        debug!("[MESSAGE] >> {:#?}", msg);
        self.sink.unbounded_send(msg)
    }

    /// Closes the connection and returns a boolean value describing whether we closed the channel.
    /// This may be false if we already closed the channel or the other side closed it.
    pub fn close(&self, ty: CloseType) -> bool {
        self.closing_helper.close(ty)
    }

    /// Checks whether a connection has been closed from our end.
    pub fn closed(&self) -> bool {
        // TODO currently this only reflects our end of the connection
        self.closing_helper.closed()
    }
}

impl Debug for PeerSink {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerSink {{}}")
    }
}

impl PartialEq for PeerSink {
    fn eq(&self, other: &PeerSink) -> bool {
        Arc::ptr_eq(&self.closing_helper, &other.closing_helper)
    }
}

impl Eq for PeerSink {}

// There is a single closing helper per connection.
// Thus, we use the hash value of its pointer here.
impl Hash for PeerSink {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let ptr = self.closing_helper.as_ref() as *const ClosingHelper;
        ptr.hash(state);
    }
}

pub enum PeerStreamEvent {
    Message(Message),
    Close(CloseType),
    Error(UniquePtr<NimiqMessageStreamError>),
}

pub struct PeerStream {
    stream: SharedNimiqMessageStream,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
}

impl PeerStream {
    pub fn new(stream: SharedNimiqMessageStream, notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>) -> Self {
        PeerStream {
            stream,
            notifier,
        }
    }

    pub fn process_stream(self) -> impl Future<Item=(), Error=NimiqMessageStreamError> + 'static {
        let stream = self.stream;
        let msg_notifier = self.notifier.clone();
        let error_notifier = self.notifier.clone();
        let close_notifier = self.notifier;

        let process_message = stream.for_each(move |msg| {
            debug!("[MESSAGE] << {:#?}", msg);
            msg_notifier.read().notify(PeerStreamEvent::Message(msg));
            Ok(())
        }).or_else(move |error| {
            debug!("[MESSAGE] Error: {:#?}", error);
            error_notifier.read().notify(PeerStreamEvent::Error(UniquePtr::new(&error)));
            Err(error)
        }).and_then(move |result| {
            close_notifier.read().notify(PeerStreamEvent::Close(CloseType::ClosedByRemote));
            Ok(result)
        });

        process_message
    }
}

impl Debug for PeerStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.stream.fmt(f)
    }
}
