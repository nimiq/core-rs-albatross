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
use futures::future;

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

        stream.for_each(move |msg| {
            match msg {
                WebSocketMessage::Message(msg) => {
                    // Ignore messages from peer if connection has been closed by us, but await close frame.
                    if !msg_closed_flag.is_closed() {
                        msg_notifier.read().notify(PeerStreamEvent::Message(msg));
                    }
                },
                WebSocketMessage::Close(_frame) => {
                    msg_closed_flag.set_closed(true);
                    let ty = msg_closed_flag.close_type().unwrap_or(CloseType::ClosedByRemote);
                    msg_notifier.read().notify(PeerStreamEvent::Close(ty));
                },
                // We have a type WebSocketMessage::Resume that is only used in the Sink and will never be returned here.
                _ => unreachable!(),
            }
            Ok(())
        }).then(move |result| {
            match result {
                Err(error) => {
                    match &error {
                        Error::WebSocketError(WsError::ConnectionClosed) => {
                            error_closed_flag.set_closed(true);
                            let ty = error_closed_flag.close_type().unwrap_or(CloseType::ClosedByRemote);
                            error_notifier.read().notify(PeerStreamEvent::Close(ty));
                        },
                        error => {
                            error_notifier.read().notify(PeerStreamEvent::Error(UniquePtr::new(error)));
                        },
                    }
                    future::err(error)
                },
                Ok(_) => {
                    // If the stream was closed without any error or close frame (just in case), call close notifier as well.
                    error_closed_flag.set_closed(true);
                    let ty = error_closed_flag.close_type().unwrap_or(CloseType::ClosedByRemote);
                    error_notifier.read().notify(PeerStreamEvent::Close(ty));
                    future::ok(())
                }
            }
        })
    }
}

impl Debug for PeerStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.stream.fmt(f)
    }
}
