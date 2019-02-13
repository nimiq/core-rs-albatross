use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::sync::mpsc::*;
use parking_lot::RwLock;

use network_messages::{Message, MessageNotifier};
use utils::observer::Notifier;

use crate::connection::close_type::CloseType;
use crate::connection::network_connection::AddressInfo;
use crate::connection::network_connection::ClosedFlag;
use crate::connection::network_connection::NetworkConnection;
#[cfg(feature = "metrics")]
use crate::network_metrics::MessageMetrics;
use crate::websocket::Message as WebSocketMessage;

use super::sink::PeerSink;
use super::stream::PeerStreamEvent;

#[derive(Clone)]
pub struct PeerChannel {
    pub msg_notifier: Arc<MessageNotifier>,
    pub close_notifier: Arc<RwLock<Notifier<'static, CloseType>>>,
    peer_sink: PeerSink,
    pub address_info: AddressInfo,
    closed_flag: ClosedFlag,

    #[cfg(feature = "metrics")]
    pub message_metrics: Arc<MessageMetrics>,
}

impl PeerChannel {
    pub fn new(network_connection: &NetworkConnection) -> Self {
        let msg_notifier = Arc::new(MessageNotifier::new());
        let close_notifier = Arc::new(RwLock::new(Notifier::new()));

        #[cfg(feature = "metrics")]
        let message_metrics = Arc::new(MessageMetrics::new());

        let msg_notifier1 = msg_notifier.clone();
        let close_notifier1 = close_notifier.clone();

        #[cfg(feature = "metrics")]
        let message_metrics1 = message_metrics.clone();

        let info = network_connection.address_info();
        let close_event_sent = AtomicBool::new(false);
        network_connection.notifier.write().register(move |e: PeerStreamEvent| {
            match e {
                PeerStreamEvent::Message(msg) => {
                    #[cfg(feature = "metrics")]
                    message_metrics1.note_message(msg.ty());
                    msg_notifier1.notify(msg)
                },
                PeerStreamEvent::Close(ty) => {
                    // Only send close event once, i.e., if close_event_sent was false.
                    if !close_event_sent.swap(true, Ordering::AcqRel) {
                        close_notifier1.read().notify(ty)
                    }
                },
                PeerStreamEvent::Error(error) => {
                    // Only send close event once, i.e., if close_event_sent was false.
                    if !close_event_sent.swap(true, Ordering::AcqRel) {
                        debug!("Stream with peer closed with error: {} ({})", error.as_ref(), info);
                        close_notifier1.read().notify(CloseType::NetworkError);
                    }
                }
            }
        });

        PeerChannel {
            msg_notifier,
            close_notifier,
            peer_sink: network_connection.peer_sink(),
            address_info: network_connection.address_info(),
            closed_flag: network_connection.closed_flag(),

            #[cfg(feature = "metrics")]
            message_metrics,
        }
    }

    pub fn send(&self, msg: Message) -> Result<(), SendError<WebSocketMessage>> {
        self.peer_sink.send(msg)
    }

    pub fn send_or_close(&self, msg: Message) {
        if self.peer_sink.send(msg).is_err() {
            let _ = self.peer_sink.close(CloseType::SendFailed, Some("SendFailed".to_string())); // TODO: We ignore the error here.
        }
    }

    pub fn closed(&self) -> bool {
        self.closed_flag.is_closed()
    }

    pub fn close(&self, ty: CloseType) -> bool {
        self.peer_sink.close(ty, None).is_ok()
    }
}

impl Debug for PeerChannel {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PeerChannel {{}}")
    }
}

impl PartialEq for PeerChannel {
    fn eq(&self, other: &PeerChannel) -> bool {
        self.peer_sink == other.peer_sink
    }

    fn ne(&self, other: &PeerChannel) -> bool {
        self.peer_sink != other.peer_sink
    }
}

impl Eq for PeerChannel {}

impl Hash for PeerChannel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.peer_sink.hash(state);
    }
}
