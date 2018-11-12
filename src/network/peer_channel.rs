use network::websocket::NimiqMessageStream;
use tokio::prelude::{Stream,Sink};
use network::message::Message;
use futures::prelude::*;
use futures::sync::mpsc::*;
use futures::stream::Forward;
use network::websocket::NimiqMessageStreamError;
use network::websocket::SharedNimiqMessageStream;
use consensus::base::primitive::hash::Argon2dHash;
use parking_lot::Mutex;
use std::sync::Arc;
use std::fmt::Debug;
use std::fmt;
use network::address::net_address::NetAddress;
use network::address::peer_address::PeerAddress;
use network::Protocol;
use network::connection::close_type::CloseType;

#[derive(Clone, Debug)]
pub struct Peer {
    sink: PeerSink,
    version: Option<u8>,
    head_hash: Option<Argon2dHash>,
    time_offset: Option<u8>,
}

impl Peer {
    pub fn new(sink: PeerSink) -> Self {
        Peer {
            sink,
            version: None,
            head_hash: None,
            time_offset: None,
        }
    }
}

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

pub struct Session {
    peer: Peer,
    protocols: Mutex<Vec<Box<Agent>>>,
}

impl Session {
    fn new(sink: PeerSink) -> Session {
        let peer = Peer::new(sink.clone());
        let ping = PingAgent::new(sink.clone()).boxed();
        Session {
            peer,
            protocols: Mutex::new(vec![ping]),
        }
    }

    pub fn initialize(&self) {
        for protocol in self.protocols.lock().iter_mut() {
            protocol.initialize();
        }
    }

//    pub fn maintain(&self) {
//        for protocol in self.protocols.lock().iter_mut() {
//            protocol.maintain();
//        }
//    }

    pub fn on_message(&self, msg: Message) -> Result<(), ProtocolError> {
        self.protocols.lock().iter_mut().map(|protocol| {
            protocol.on_message(&msg)
        })
            .collect::<Result<Vec<()>, ProtocolError>>()
            .map(|_| ())
    }

    pub fn on_close(&self) {
        for protocol in self.protocols.lock().iter_mut() {
            protocol.on_close();
        }
    }
}

impl Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Session {{ peer: {:?} }}", self.peer)
    }
}

pub struct PeerSink {
    sink: UnboundedSender<Message>
}

impl PeerSink {
    pub fn new(channel: UnboundedSender<Message>) -> Self {
        PeerSink {
            sink: channel.clone()
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<Message>> {
        self.sink.unbounded_send(msg)
    }
}

impl Clone for PeerSink {
    fn clone(&self) -> Self {
        PeerSink {
            sink: self.sink.clone()
        }
    }
}

impl Debug for PeerSink {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerSink {{}}")
    }
}

#[derive(Debug)]
pub struct PeerStream {
    stream: SharedNimiqMessageStream,
    session: Arc<Session>,
}

impl PeerStream {
    pub fn new(stream: SharedNimiqMessageStream, session: Arc<Session>) -> Self {
        PeerStream {
            stream,
            session,
        }
    }

    pub fn process_stream(self) -> impl Future<Item=(), Error=NimiqMessageStreamError> {
        let stream = self.stream;
        let session = self.session;

        let process_message = stream.for_each(move |msg| {
            if let Err(err) = session.on_message(msg) {
                println!("{:?}", err);
                // TODO: What to do with the error here?
            }
            Ok(())
        });

        process_message
    }
}

pub struct PeerConnection {
    peer_stream: Option<PeerStream>,
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    forward_future: Option<Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>>,
    session: Arc<Session>,
//    pub peer_address: PeerAddress,
}

impl PeerConnection {
    pub fn new(stream: NimiqMessageStream) -> Self {
        let shared_stream: SharedNimiqMessageStream = stream.into();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = Some(rx.forward(shared_stream.clone()));

        let peer_sink = PeerSink::new(tx);
        let session = Arc::new(Session::new(peer_sink.clone()));

        PeerConnection {
            peer_stream: Some(PeerStream::new(shared_stream.clone(), session.clone())),
            peer_sink,
            stream: shared_stream,
            forward_future,
            session,
        }
    }

    pub fn process_connection(&mut self) -> impl Future<Item=(), Error=()> {
        assert!(self.forward_future.is_some() && self.peer_stream.is_some(), "Process connection can only be called once!");

        self.session.initialize();

        let forward_future = self.forward_future.take().unwrap();
        let stream = self.peer_stream.take().unwrap();
        let pair = forward_future.join(stream.process_stream().map_err(|_| ())); // TODO: throwing away error info here
        pair.map(|_| ())
    }

    pub fn close(&mut self, ty: CloseType, reason: &str) -> Poll<(), ()> {
        debug!("Closing {:?} connection, reason: {}", ty, reason);
        self.session.on_close();
        self.stream.close()
    }

    pub fn net_address(&self) -> &NetAddress {
        self.stream.net_address()
    }

    pub fn outbound(&self) -> bool {
        self.stream.outbound()
    }

    pub fn inbound(&self) -> bool {
        !self.outbound()
    }
}
