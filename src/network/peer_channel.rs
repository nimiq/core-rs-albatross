use network::websocket::NimiqMessageStream;
use tokio::prelude::{Stream,Sink};
use network::message::Message;
use network::websocket::IntoData;
use futures::prelude::*;
use tokio::prelude::stream::{SplitStream,SplitSink};
use tokio::run;
use futures::sync::mpsc::*;

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

    pub fn run(&self) {
//        let process_message = self.stream.for_each(move |msg| {
//             self.session.on_message(msg);
//             Ok(())
//         });

        // run(process_message.map(|_| ()).map_err(|_| ()));
        unimplemented!();
    }
}

pub struct Network {
    peer_stream: PeerStream,
    peer_sink: PeerSink,
}

impl Network {
    pub fn new(stream: NimiqMessageStream) -> Self {
        let (sink, stream) = stream.split();
        let (tx, rx) = unbounded(); // TODO: use bounded channel?

        // TODO: returned future is discarded right now...
        rx.forward(sink);

        Network {
            peer_stream: PeerStream::new(stream),
            peer_sink: PeerSink::new(tx),
        }
    }
}
