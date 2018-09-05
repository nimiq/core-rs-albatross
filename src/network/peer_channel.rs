use network::websocket::NimiqMessageStream;
use tokio::prelude::{Stream,Sink};
use network::message::Message;
use network::websocket::IntoData;
use futures::prelude::*;
use tokio::prelude::stream::{SplitStream,SplitSink};
use tokio::run;

pub struct Session {}

impl Session {
    pub fn on_message(&self, msg: Message) {

    }
}

pub struct PeerInfo {}

pub struct PeerSink {
    
}

pub struct PeerStream {
    stream: SplitStream<NimiqMessageStream>,
    session: Session,
}

impl PeerStream {
    pub fn new(stream: SplitStream<NimiqMessageStream>) -> PeerStream {
        let session = Session {};
        PeerStream {
            stream,
            session,
        }
    }

    pub fn run(&self) {
//        let (sink, stream) = self.stream.split();

        // let process_message = self.stream.for_each(move |msg| {
        //     self.session.on_message(msg);
        //     Ok(())
        // });
        // let process_message = process_message.then(|_| Ok(()));

        // run(process_message.map(|_| ()).map_err(|_| ()));
        unimplemented!();
    }
}
