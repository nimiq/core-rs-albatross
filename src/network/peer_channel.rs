use network::websocket::NimiqMessageStream;
use tokio::prelude::{Stream,Sink};
use network::message::Message;
use network::websocket::IntoData;
use futures::prelude::*;
use tokio::prelude::stream::{SplitStream,SplitSink};
use tokio::run;
use futures::sync::mpsc::*;
use futures::stream::Forward;
use network::websocket::NimiqMessageStreamError;

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
    stream: SplitStream<NimiqMessageStream>,
    session: Session,
}

impl PeerStream {
    pub fn new(stream: SplitStream<NimiqMessageStream>) -> Self {
        let session = Session {};
        PeerStream {
            stream,
            session,
        }
    }

    pub fn run(self) -> impl Future<Item=(), Error=NimiqMessageStreamError> {
        let stream = self.stream;
        let session = self.session;

        let process_message = stream.for_each(move |msg| {
            // TODO: should we spawn a new task on tokio here already?
            session.on_message(msg);
            Ok(())
        });

        process_message
    }
}

pub struct Network {
    peer_stream: PeerStream,
    peer_sink: PeerSink,
    forward_future: Forward<UnboundedReceiver<Message>, SplitSink<NimiqMessageStream>>
}

impl Network {
    pub fn new(stream: NimiqMessageStream) -> Self {
        let (sink, stream) = stream.split();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        let forward_future = rx.forward(sink);

        Network {
            peer_stream: PeerStream::new(stream),
            peer_sink: PeerSink::new(tx),
            forward_future
        }
    }

    pub fn run(self) -> impl Future<Item=(), Error=()> {
        let forward_future = self.forward_future;
        let stream = self.peer_stream;
        let pair = forward_future.join(stream.run().map_err(|_| ())); // TODO: throwing away error info here
        pair.map(|_| ())
    }
}
