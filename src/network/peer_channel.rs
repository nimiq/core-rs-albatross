use network::websocket::NimiqMessageStream;
use tokio::prelude::{Stream,Sink};
use network::message::Message;
use futures::prelude::*;
use futures::sync::mpsc::*;
use futures::stream::Forward;
use network::websocket::NimiqMessageStreamError;
use network::websocket::SharedNimiqMessageStream;

pub struct Session {}

impl Session {
    pub fn on_message(&self, msg: Message) {

    }
}

pub struct PeerInfo {}

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

pub struct PeerStream {
    stream: SharedNimiqMessageStream,
    session: Session,
}

impl PeerStream {
    pub fn new(stream: SharedNimiqMessageStream) -> Self {
        let session = Session {};
        PeerStream {
            stream,
            session,
        }
    }

    pub fn process_stream(self) -> impl Future<Item=(), Error=NimiqMessageStreamError> {
        let stream = self.stream;
        let session = self.session;

        let process_message = stream.for_each(move |msg| {
            session.on_message(msg);
            Ok(())
        });

        process_message
    }
}

pub struct PeerConnection {
    peer_stream: Option<PeerStream>,
    peer_sink: PeerSink,
    stream: SharedNimiqMessageStream,
    forward_future: Option<Forward<UnboundedReceiver<Message>, SharedNimiqMessageStream>>
}

impl PeerConnection {
    pub fn new(stream: NimiqMessageStream) -> Self {
        let shared_stream: SharedNimiqMessageStream = stream.into();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = Some(rx.forward(shared_stream.clone()));

        PeerConnection {
            peer_stream: Some(PeerStream::new(shared_stream.clone())),
            peer_sink: PeerSink::new(tx),
            stream: shared_stream,
            forward_future,
        }
    }

    pub fn process_connection(&mut self) -> impl Future<Item=(), Error=()> {
        assert!(self.forward_future.is_some() && self.peer_stream.is_some(), "Process connection can only be called once!");

        let forward_future = self.forward_future.take().unwrap();
        let stream = self.peer_stream.take().unwrap();
        let pair = forward_future.join(stream.process_stream().map_err(|_| ())); // TODO: throwing away error info here
        pair.map(|_| ())
    }

    pub fn close(&mut self) -> Poll<(), ()> {
        self.stream.close()
    }
}
