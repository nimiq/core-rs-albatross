use std::sync::Arc;

use futures::prelude::*;
use futures::stream::Forward;
use futures::sync::mpsc::*;

use network::address::net_address::NetAddress;
use network::connection::close_type::CloseType;
use network::message::Message;
use network::peer_channel::PeerSink;
use network::peer_channel::PeerStream;
use network::peer_channel::Session;
use network::websocket::NimiqMessageStream;
use network::websocket::SharedNimiqMessageStream;
use network::address::peer_address::PeerAddress;

pub struct NetworkConnection {
    peer_stream: Option<PeerStream>,
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    forward_future: Option<Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>>,
    session: Arc<Session>,
    peer_address: Option<PeerAddress>,
}

impl NetworkConnection {
    pub fn new(stream: NimiqMessageStream, peer_address: Option<PeerAddress>) -> Self {
        let shared_stream: SharedNimiqMessageStream = stream.into();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = Some(rx.forward(shared_stream.clone()));

        let peer_sink = PeerSink::new(tx);
        let session = Arc::new(Session::new(peer_sink.clone()));

        NetworkConnection {
            peer_stream: Some(PeerStream::new(shared_stream.clone(), session.clone())),
            peer_sink,
            stream: shared_stream,
            forward_future,
            session,
            peer_address
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

    pub fn peer_address(&self) -> Option<&PeerAddress> { self.peer_address.as_ref() }
    pub fn set_peer_address(&mut self, peer_address: PeerAddress) {
        self.peer_address = Some(peer_address)
    }
}
