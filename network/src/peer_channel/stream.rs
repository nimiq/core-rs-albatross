use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;

use futures::prelude::*;
use parking_lot::RwLock;
use tokio::prelude::Stream;
use tungstenite::error::Error as WsError;

use network_messages::Message;
use utils::observer::PassThroughNotifier;
use utils::unique_ptr::UniquePtr;

use crate::connection::close_type::CloseType;
use crate::connection::network_connection::ClosedFlag;
use crate::websocket::{Error, SharedNimiqMessageStream};
use crate::websocket::Message as WebSocketMessage;

pub enum PeerStreamEvent {
    Message(Message),
    Close(CloseType),
    Error(UniquePtr<Error>),
}

pub struct PeerStream {
    stream: SharedNimiqMessageStream,
    closed_flag: ClosedFlag,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>,
}

impl PeerStream {
    pub fn new(stream: SharedNimiqMessageStream, notifier: Arc<RwLock<PassThroughNotifier<'static, PeerStreamEvent>>>, closed_flag: ClosedFlag) -> Self {
        PeerStream {
            stream,
            notifier,
            closed_flag,
        }
    }

    pub fn process_stream(self) -> impl Future<Item=(), Error=Error> + 'static {
        let stream = self.stream;
        let msg_notifier = self.notifier.clone();
        let error_notifier = self.notifier;
        let msg_closed_flag = self.closed_flag.clone();
        let error_closed_flag = self.closed_flag;

        let process_message = stream.for_each(move |msg| {
            match msg {
                WebSocketMessage::Message(msg) => {
                    msg_notifier.read().notify(PeerStreamEvent::Message(msg));
                },
                WebSocketMessage::Close(frame) => {
                    msg_closed_flag.update(true);
                    msg_notifier.read().notify(PeerStreamEvent::Close(frame.map(|f| f.code).into()));
                },
            }
            Ok(())
        }).or_else(move |error| {
            match &error {
                Error::WebSocketError(WsError::ConnectionClosed(ref frame)) => {
                    error_closed_flag.update(true);
                    error_notifier.read().notify(PeerStreamEvent::Close(frame.as_ref().map(|f| f.code).into()));
                },
                error => {
                    error_notifier.read().notify(PeerStreamEvent::Error(UniquePtr::new(error)));
                },
            }
            Err(error)
        });

        process_message
    }
}

impl Debug for PeerStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.stream.fmt(f)
    }
}
