use std::collections::HashMap;
use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::hash::Hasher;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Instant;

use async_trait::async_trait;
use atomic::Atomic;
use futures::sync::mpsc::*;
use futures_03::{
    executor,
    task::{noop_waker_ref, Context},
    Sink, SinkExt, Stream,
};
use parking_lot::RwLock;

use beserial::Deserialize;
use network_interface::message::{peek_type, Message};
use network_interface::peer::dispatch::{unbounded_dispatch, DispatchError};
use network_interface::peer::{CloseReason, Peer as PeerInterface, SendError as SendErrorI, RequestResponse};
use network_messages::MessageNotifier;
use peer_address::address::PeerAddress;
use utils::observer::Notifier;

use crate::connection::close_type::CloseType;
use crate::connection::network_connection::AddressInfo;
use crate::connection::network_connection::ClosedFlag;
use crate::connection::network_connection::NetworkConnection;
#[cfg(feature = "metrics")]
use crate::network_metrics::MessageMetrics;
use crate::websocket::Message as WebSocketMessage;
use crate::network::NetworkError;

use super::sink::PeerSink;
use super::stream::PeerStreamEvent;

pub type Channels = Arc<RwLock<HashMap<u64, Pin<Box<dyn Sink<Vec<u8>, Error = DispatchError> + Send + Sync>>>>>;

#[derive(Clone)]
pub struct PeerChannel {
    pub msg_notifier: Arc<MessageNotifier>,
    pub close_notifier: Arc<RwLock<Notifier<'static, CloseType>>>,
    pub(crate) peer_sink: PeerSink,
    pub address_info: AddressInfo,
    closed_flag: ClosedFlag,
    pub last_message_received: Arc<Atomic<Instant>>,
    close_event_sent: Arc<AtomicBool>,

    // TODO: Use Mutex instead?
    msg_channels: Channels,

    #[cfg(feature = "metrics")]
    pub message_metrics: Arc<MessageMetrics>,
}

impl PeerChannel {
    pub fn new(network_connection: &NetworkConnection) -> Self {
        let msg_notifier = Arc::new(MessageNotifier::new());
        let close_notifier = Arc::new(RwLock::new(Notifier::new()));
        let msg_channels: Channels = Arc::new(RwLock::new(HashMap::new()));

        #[cfg(feature = "metrics")]
        let message_metrics = Arc::new(MessageMetrics::new());
        let last_message_received = Arc::new(Atomic::new(Instant::now()));

        let msg_notifier1 = msg_notifier.clone();
        let close_notifier1 = close_notifier.clone();
        let last_message_received1 = last_message_received.clone();
        let msg_channels1 = msg_channels.clone();

        let info = network_connection.address_info();
        let close_event_sent = Arc::new(AtomicBool::new(false));
        let close_event_sent_inner = close_event_sent.clone();
        network_connection.notifier.write().register(move |e: PeerStreamEvent| {
            match e {
                PeerStreamEvent::Message(msg) => {
                    last_message_received1.store(Instant::now(), Ordering::Relaxed);

                    if let Ok(msg_type) = peek_type(&msg) {
                        // There are 3 possibilities:
                        // 1. We have a message channel, then we send it via this channel.
                        // 2. We don't have a channel, but it is a legacy message.
                        //    Then we send it via this channel.
                        // 3. None of the above is true. Then, we discard the message.
                        let mut channels = msg_channels1.write();
                        if let Some(channel) = channels.get_mut(&msg_type) {
                            // TODO: For now, we ignore asyncness here and block.
                            if executor::block_on(channel.send(msg)).is_err() {
                                // TODO: What to do if there is an error?
                            }
                        } else {
                            // Test whether it is a legacy message.
                            if let Ok(msg) = Deserialize::deserialize_from_vec(&msg) {
                                msg_notifier1.notify(msg);
                            }
                        }
                    } else {
                        // TODO: What to do if we cannot parse the message type?
                    }
                }
                PeerStreamEvent::Close(ty) => {
                    // Only send close event once, i.e., if close_event_sent was false.
                    if !close_event_sent_inner.swap(true, Ordering::AcqRel) {
                        close_notifier1.read().notify(ty)
                    }
                }
                PeerStreamEvent::Error(error) => {
                    // Only send close event once, i.e., if close_event_sent was false.
                    if !close_event_sent_inner.swap(true, Ordering::AcqRel) {
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
            last_message_received,
            close_event_sent,

            msg_channels,

            #[cfg(feature = "metrics")]
            message_metrics,
        }
    }

    pub fn send<M: Message>(&self, msg: M) -> Result<(), SendError<WebSocketMessage>> {
        self.peer_sink.send(&msg)
    }

    pub fn send_or_close<M: Message>(&self, msg: M) {
        if self.peer_sink.send(&msg).is_err() {
            self.peer_sink.close(CloseType::SendFailed, Some("SendFailed".to_string()));
        }
    }

    pub fn receive<T: Message + 'static>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        let (tx, rx) = unbounded_dispatch();

        let mut channels = self.msg_channels.write();
        // Check if receiver has been dropped if one already exists.
        if let Some(channel) = channels.get_mut(&T::TYPE_ID) {
            let mut cx = Context::from_waker(noop_waker_ref());
            // We expect the sink to return an error because the receiver is dropped.
            // If not, we would overwrite an active receive.
            match channel.as_mut().poll_ready(&mut cx) {
                Poll::Ready(Err(_)) => {}
                _ => panic!("Receiver for message type {} already exists", T::TYPE_ID),
            }
        }
        channels.insert(T::TYPE_ID, tx);

        rx
    }

    pub fn closed(&self) -> bool {
        self.closed_flag.is_closed()
    }

    pub fn close(&self, ty: CloseType) {
        self.peer_sink.close(ty, None);
        let notifier = self.close_notifier.clone();
        let close_event_sent = self.close_event_sent.clone();
        tokio::spawn(futures::lazy(move || {
            // Only send close event once, i.e., if close_event_sent was false.
            if !close_event_sent.swap(true, Ordering::AcqRel) {
                notifier.read().notify(ty);
            }
            futures::future::ok(())
        }));
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
}

impl Eq for PeerChannel {}

impl Hash for PeerChannel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.peer_sink.hash(state);
    }
}

#[async_trait]
impl PeerInterface for PeerChannel {
    type Id = Arc<PeerAddress>;
    type Error = NetworkError;

    fn id(&self) -> Self::Id {
        self.address_info.peer_address().expect("PeerAddress not set")
    }

    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendErrorI> {
        self.peer_sink.send(msg).map_err(|_| SendErrorI::AlreadyClosed)
    }

    fn receive<T: Message + 'static>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        self.receive()
    }

    fn close(&self, _ty: CloseReason) {
        self.close(CloseType::Unknown);
    }

    async fn request<R: RequestResponse>(&self, _request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}
