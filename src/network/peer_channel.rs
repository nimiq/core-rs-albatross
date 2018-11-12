use std::fmt;
use std::fmt::Debug;
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
    pub fn new(sink: PeerSink) -> Session {
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

#[derive(Clone)]
pub struct PeerSink {
    sink: UnboundedSender<Message>,
    pub address_info: AddressInfo,
}

impl PeerSink {
    pub fn new(channel: UnboundedSender<Message>, address_info: AddressInfo) -> Self {
        PeerSink {
            sink: channel.clone(),
            address_info
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
