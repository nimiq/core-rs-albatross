use network::websocket::NimiqMessageStream;
use tokio::prelude::{Stream,Sink};
use network::message::Message;
use network::websocket::IntoData;
use futures::prelude::*;
use tokio::run;

pub struct Session {}

impl Session {
    pub fn on_message(&self, msg: Message) {

    }
}

pub struct PeerInfo {}

pub struct PeerChannel<S: Stream + Sink + Send + Sync>
    where S::Item: IntoData + Send + Sync {
    stream: NimiqMessageStream<S>,
    session: Session,
    peerInfo: PeerInfo,
}

impl<S: Stream + Sink + Send + Sync> PeerChannel<S>
    where S::Item: IntoData + Send + Sync {
    pub fn new(stream: NimiqMessageStream<S>, peerInfo: PeerInfo) -> PeerChannel<S> {
        let session = Session {};
        PeerChannel {
            stream,
            session,
            peerInfo
        }
    }

    pub fn run(&self) {
//        let (sink, stream) = self.stream.split();

        let process_message = self.stream.for_each(move |msg| {
            self.session.on_message(msg);
            Ok(())
        });
        let process_message = process_message.then(|_| Ok(()));

        run(process_message.map(|_| ()).map_err(|_| ()));
    }
}
