use std::collections::VecDeque;
use std::task::Waker;
use std::{collections::HashMap, pin::Pin, sync::Arc};

use bytes::{Buf, Bytes};
use futures::{
    channel::mpsc,
    io::{AsyncRead, AsyncWrite},
    ready,
    sink::Sink,
    stream::{Stream, StreamExt},
    task::{Context, Poll},
};
use tokio_util::codec::Framed;

use beserial::{Deserialize, Serialize};

use crate::codecs::{
    tokio_adapter::TokioAdapter,
    typed::{Error, Message, MessageCodec, MessageType},
};
use crate::peer::Peer;

type FramedStream<C> = Framed<TokioAdapter<C>, MessageCodec>;

pub trait SendMessage<S>: Send + Sync {
    fn send(self: Box<Self>, sink: Pin<&mut S>) -> Result<(), Error>;
}

impl<S, F: FnOnce(Pin<&mut S>) -> Result<(), Error>> SendMessage<S> for F
where
    F: Send + Sync,
{
    fn send(self: Box<F>, sink: Pin<&mut S>) -> Result<(), Error> {
        self(sink)
    }
}

/// Message dispatcher for a single socket.
///
/// This sends messages to the peer and receives messages from the peer.
///
/// Messages are received by calling `poll_incoming`, which reads messages from the underlying socket and
/// pushes them into a registered stream. Exactly one stream can be registered per message type. Receiver
/// streams can be registered by calling `receive`.
///
/// If no stream is registered for a message type, and a message of that type is received, it will be
/// buffered, and once a stream is registered it will read the buffered messages first (in order as they were
/// received).
///
/// # TODO
///
///  - Something requires the underlying stream `C` to be be pinned, but I'm not sure what. I think we can
///    just pin it to the heap - it'll be fine...
///
pub struct MessageDispatch<C>
where
    C: AsyncRead + AsyncWrite + Send + Sync,
{
    framed: Pin<Box<FramedStream<C>>>,

    /// Channels that receive raw messages for a specific message type.
    ///
    /// Note: Those are ignored if the peer was not set for this dispatch.
    channels: HashMap<MessageType, mpsc::Sender<(Bytes, Arc<Peer>)>>,

    /// A single buffer slot. This is needed because we can only find out if we have capacity for a message after
    /// receiving it.
    buffer: Option<(MessageType, Bytes)>,

    /// The buffer size for new channels.
    channel_size: usize,

    outbound_messages: VecDeque<Box<dyn SendMessage<FramedStream<C>>>>,

    waker: Option<Waker>,
}

impl<C> MessageDispatch<C>
where
    C: AsyncRead + AsyncWrite + Send + Sync + 'static + Unpin,
{
    ///
    /// # Arguments
    ///
    ///  - `socket`: The underlying socket
    ///  - `max_buffered`: Maximum number of buffered messages. Must be at least 1.
    ///
    pub fn new(socket: C, channel_size: usize) -> Self {
        Self {
            framed: Box::pin(Framed::new(
                TokioAdapter::new(socket),
                MessageCodec::default(),
            )),
            channels: HashMap::new(),
            buffer: None,
            channel_size,
            outbound_messages: VecDeque::new(),
            waker: None,
        }
    }

    pub fn send<M: Message>(&mut self, message: M) -> Result<(), Error> {
        self.outbound_messages
            .push_back(Box::new(move |sink: Pin<&mut FramedStream<C>>| {
                Sink::<&M>::start_send(sink, &message)
            }));
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
        Ok(())
    }

    /// Polls the inbound socket and either pushes the message to the registered channel, or buffers it.
    ///
    pub fn poll_inbound(
        &mut self,
        cx: &mut Context<'_>,
        peer: &Arc<Peer>,
    ) -> Poll<Result<(), Error>> {
        loop {
            // Try to dispatch the buffered value. This will return Poll::Pending if the buffer can't be cleared.
            if let Some((type_id, _)) = &self.buffer {
                let type_id = *type_id;

                if let Some(tx) = self.channels.get_mut(&type_id) {
                    let mut receiver_is_gone = false;

                    match tx.poll_ready(cx) {
                        // No space to put the message into the channel
                        Poll::Pending => return Poll::Pending,

                        // Send error - the receiver must have been closed.
                        Poll::Ready(Err(e)) => {
                            self.buffer.take(); // Drop message
                            log::debug!(
                                "Receiver returned Err({:?}) for poll_ready() -> Dropping message: type_id={}, peer={}",
                                e,
                                type_id,
                                peer.id,
                            );
                            receiver_is_gone = true;
                        }

                        // We have space, so send the message to the channel.
                        Poll::Ready(Ok(())) => {
                            // Take the buffered message. We know that there is one, from the outer `if let Some`-block
                            let (_, data) = self.buffer.take().unwrap();

                            // Not sure why this still can fail, but if it does, we consider the receiver to be gone.
                            if let Err(e) = tx.start_send((data, Arc::clone(peer))) {
                                log::debug!(
                                    "poll_ready() was Ok(), but send is not: Err({:?}) -> Dropping message: type_id={}, peer={}",
                                    e,
                                    type_id,
                                    peer.id,
                                );
                                receiver_is_gone = true
                            }
                        }
                    }

                    if receiver_is_gone {
                        // Send error, i.e. the receiver is closed. Remove it, but usually we expect receivers to stay around.
                        self.channels.remove(&type_id);
                    }
                } else {
                    // Drop message
                    log::trace!(
                        "No receiver for message type. Dropping message: type_id={}, peer={}",
                        type_id,
                        peer.id
                    );
                    self.buffer.take();
                }
            }

            // Poll the incoming stream and handle the message
            match self.framed.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok((type_id, data)))) => {
                    // A message was received. The stream gives us tuples of message type and data (BytesMut)
                    // Store the message into the buffer and continue the loop (i.e. immediately trying to send it to the
                    // receivers).
                    assert!(self.buffer.is_none());

                    // We 'freeze' the message, i.e. turning the `BytesMut` into a `Bytes`. We could use this to cheaply
                    // clone the reference to the data.
                    self.buffer = Some((type_id, data.freeze()));
                }

                // Error while receiving a message. This could be an error from the underlying socket (i.e. an
                // IO error), or the message was malformed.
                Poll::Ready(Some(Err(e))) => {
                    log::warn!("socket error: {}", e);
                    return Poll::Ready(Err(e));
                }

                // End of stream. So we terminate the future
                Poll::Ready(None) => {
                    log::warn!("end of stream");
                    return Poll::Ready(Ok(()));
                }

                // We need to wait for more data
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }

    pub fn poll_outbound(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        store_waker!(self, waker, cx);

        // We need to call poll_close for a specific Sink<T>, so...
        #[derive(Debug, Serialize, Deserialize)]
        struct CompilerShutUp;
        impl Message for CompilerShutUp {
            const TYPE_ID: u64 = 420;
        }

        while ready!(Sink::<&CompilerShutUp>::poll_ready(
            self.framed.as_mut(),
            cx
        ))
        .is_ok()
        {
            if let Some(send_message) = self.outbound_messages.pop_front() {
                if let Err(e) = send_message.send(self.framed.as_mut()) {
                    return Poll::Ready(Err(e));
                }
            } else {
                break;
            }
        }

        Sink::<&CompilerShutUp>::poll_flush(self.framed.as_mut(), cx)
    }

    pub fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        // We need to call poll_close for a specific Sink<T>, so...
        #[derive(Debug, Serialize, Deserialize)]
        struct CompilerShutUp;
        impl Message for CompilerShutUp {
            const TYPE_ID: u64 = 420;
        }

        {
            let sink: Pin<&mut Framed<TokioAdapter<C>, MessageCodec>> = Pin::new(&mut self.framed);
            match Sink::<&CompilerShutUp>::poll_flush(sink, cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        {
            let sink: Pin<&mut Framed<TokioAdapter<C>, MessageCodec>> = Pin::new(&mut self.framed);
            Sink::<&CompilerShutUp>::poll_close(sink, cx).map_err(|e| e)
        }
    }

    /// Registers a message receiver for a specific message type (as defined by the implementation `M`).
    ///
    /// # Panics
    ///
    /// Panics if a message receiver for this message type is already registered.
    ///
    /// # TODO
    ///
    /// Why does `M` need to be `Unpin`?
    ///
    pub fn receive<M: Message>(&mut self) -> impl Stream<Item = M> {
        let type_id = M::TYPE_ID.into();

        if self.channels.contains_key(&type_id) {
            panic!(
                "Receiver for {} message (type_id = {}) already registered",
                std::any::type_name::<M>(),
                type_id
            );
        }

        // We don't really need buffering here, since we already have that in the `MessageDispatch`.
        let (tx, rx) = mpsc::channel(self.channel_size);

        // Insert sender into channels
        self.channels.insert(M::TYPE_ID.into(), tx);

        rx.filter_map(|(data, peer)| async move {
            match Deserialize::deserialize(&mut data.reader()) {
                Ok(message) => Some(message),
                Err(e) => {
                    log::warn!(
                        "Error deserializing {} message from {}: {}",
                        std::any::type_name::<M>(),
                        peer.id,
                        e
                    );
                    None
                }
            }
        })
    }

    /// Add multiple receivers, that will receive the raw data alongside the peer.
    ///
    /// This doesn't check if a receiver for a message type is registered twice.
    ///
    pub fn receive_multiple_raw(
        &mut self,
        receive_from_all: impl IntoIterator<Item = (MessageType, mpsc::Sender<(Bytes, Arc<Peer>)>)>,
    ) {
        // todo remove stale sender
        self.channels.extend(receive_from_all);
    }

    /// remove a receiver for the peer
    pub fn remove_receiver_raw(&mut self, type_id: MessageType) {
        self.channels.remove(&type_id);
    }
}

impl<C> MessageDispatch<C>
where
    C: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    pub fn into_inner(self) -> C {
        Pin::into_inner(self.framed).into_inner().into_inner()
    }
}
