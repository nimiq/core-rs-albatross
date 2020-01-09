use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;

use futures::prelude::*;
use parking_lot::RwLock;
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

    // process_stream forwards all messages to the notifier,
    // or closes the connection if a close frame is received.
    pub async fn process_stream(mut self) -> Result<(), Error> {
        let msg_notifier = self.notifier.clone();
        let error_notifier = self.notifier;
        let msg_closed_flag = self.closed_flag.clone();
        let error_closed_flag = self.closed_flag;

        while let Some(result) = self.stream.next().await {
            match result {
                Ok(msg) => match msg {
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
                },
                Err(error) => {
                    match &error {
                        Error::WebSocketError(WsError::ConnectionClosed) => {
                            error_closed_flag.set_closed(true);
                            let ty = error_closed_flag.close_type().unwrap_or(CloseType::ClosedByRemote);
                            error_notifier.read().notify(PeerStreamEvent::Close(ty));

                        },
                        error => {
                            error_notifier.read().notify(PeerStreamEvent::Error(UniquePtr::new(error)));
                        }
                    }
                    return Err(error);
                },
            }
        }

        // If the stream was closed without any error or close frame (just in case), call close notifier as well.
        error_closed_flag.set_closed(true);
        let ty = error_closed_flag.close_type().unwrap_or(CloseType::ClosedByRemote);
        error_notifier.read().notify(PeerStreamEvent::Close(ty));
        Ok(())
    }
}

impl Debug for PeerStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.stream.fmt(f)
    }
}
