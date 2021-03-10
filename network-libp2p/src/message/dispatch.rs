use std::{collections::HashMap, pin::Pin, sync::Arc};

use bytes::{buf::BufExt, Bytes};
use futures::{
    channel::mpsc,
    future::Future,
    io::{AsyncRead, AsyncWrite},
    sink::Sink,
    stream::{Stream, StreamExt},
    task::{Context, Poll},
};
use parking_lot::Mutex;
use tokio_util::codec::Framed;

use beserial::{Deserialize, Serialize};

use crate::codecs::{
    tokio_adapter::TokioAdapter,
    typed::{Error, Message, MessageCodec, MessageType},
};

use super::peer::Peer;

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
    framed: Pin<Box<Framed<TokioAdapter<C>, MessageCodec>>>,

    /// Channels that receive raw messages for a specific message type.
    ///
    /// Note: Those are ignored if the peer was not set for this dispatch.
    channels: HashMap<MessageType, mpsc::Sender<(Bytes, Arc<Peer>)>>,

    /// A single buffer slot. This is needed because we can only find out if we have capacity for a message after
    /// receiving it.
    buffer: Option<(MessageType, Bytes)>,

    /// The size for new channels
    channel_size: usize,
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
        }
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
                log::trace!(
                    "dispatch buffered value: type_id={}, peer={:?}",
                    type_id,
                    peer
                );

                let type_id = *type_id;

                if let Some(tx) = self.channels.get_mut(&type_id) {
                    let mut receiver_is_gone = false;

                    match tx.poll_ready(cx) {
                        // No space to put the message into the channel
                        Poll::Pending => return Poll::Pending,

                        // Send error - the receiver must have been closed.
                        Poll::Ready(Err(_)) => receiver_is_gone = true,

                        // We have space, so send the message to the channel.
                        Poll::Ready(Ok(())) => {
                            // Take the buffered message. We know that there is one, from the outer `if let Some`-block
                            let (_, data) = self.buffer.take().unwrap();

                            log::trace!("dispatching message to receiver: {:?}", data);

                            // Not sure why this still can fail, but if it does, we consider the receiver to be gone.
                            if tx.start_send((data, Arc::clone(peer))).is_err() {
                                receiver_is_gone = true
                            }
                        }
                    }

                    if receiver_is_gone {
                        // Send error, i.e. the receiver is closed. Remove it, but usually we expect receivers to stay around.
                        log::warn!("Receiver is gone: type_id={}", type_id);
                        self.channels.remove(&type_id);
                        return Poll::Pending;
                    }
                } else {
                    // Drop message
                    log::warn!(
                        "No receiver for message type. Dropping message: type_id={}",
                        type_id
                    );
                    return Poll::Pending;
                }
            }

            // Poll the incoming stream and handle the message
            match self.framed.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok((type_id, data)))) => {
                    // A message was received. The stream gives us tuples of message type and data (BytesMut)
                    // Store the message into the buffer and continue the loop (i.e. immediately trying to send it to the
                    // receivers).

                    log::trace!("received message: type_id={}, data={:?}", type_id, data);

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
                    log::trace!("inbound socket pending");
                    return Poll::Pending;
                }
            }
        }
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
            tracing::trace!("flushing");
            match Sink::<&CompilerShutUp>::poll_flush(sink, cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        {
            let sink: Pin<&mut Framed<TokioAdapter<C>, MessageCodec>> = Pin::new(&mut self.framed);
            tracing::trace!("closing");
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
                "Local receiver for channel already registered: type_id = {}",
                type_id
            );
        }

        // We don't really need buffering here, since we already have that in the `MessageDispatch`.
        // TODO: Remove magic number
        let (tx, rx) = mpsc::channel(self.channel_size);

        // Insert sender into channels
        self.channels.insert(M::TYPE_ID.into(), tx);

        rx.filter_map(|(data, _peer)| async move {
            match Deserialize::deserialize(&mut data.reader()) {
                Ok(message) => Some(message),
                Err(e) => {
                    log::error!("Error while deserializing message for receiver: {}", e);
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

/// Future that sends a message over this socket.
pub struct SendMessage<'m, C, M>
where
    C: AsyncRead + AsyncWrite + Send + Sync,
{
    dispatch: Arc<Mutex<MessageDispatch<C>>>,
    message: Option<&'m M>,
}

impl<'m, C, M> SendMessage<'m, C, M>
where
    C: AsyncRead + AsyncWrite + Send + Sync,
{
    /// Creates a future that sends the message.
    ///
    /// # Arguments
    ///
    ///  - `dispatch`: An `Arc<Mutex<_>>` of the `MessageDispatch` that is used to send the message.
    ///  - `message`: A borrow of the message.
    pub fn new(dispatch: Arc<Mutex<MessageDispatch<C>>>, message: &'m M) -> Self {
        SendMessage {
            dispatch,
            message: Some(message),
        }
    }
}

impl<'m, C, M> Future for SendMessage<'m, C, M>
where
    C: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    M: Message,
{
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        //let sink: Pin<&mut Framed<TokioAdapter<C>, MessageCodec>> = Pin::new(&mut dispatch.framed);

        // If we haven't already sent the message
        if self.message.is_some() {
            // First poll sink until it's ready
            {
                tracing::trace!("polling sink to be ready");

                let mut dispatch = self.dispatch.lock();
                let sink = Pin::new(&mut dispatch.framed);

                match Sink::<&M>::poll_ready(sink, cx) {
                    // Ready, so continue.
                    Poll::Ready(Ok(())) => {}

                    // Either pending or error, so just return that
                    p => return p,
                }
            }

            // Start sending
            {
                // This always gives us a message, since the outer if-block checks for it.
                let message = self.message.take().unwrap();

                tracing::trace!(message = ?message, "start sending");

                let mut dispatch = self.dispatch.lock();
                let sink = Pin::new(&mut dispatch.framed);

                if let Err(e) = sink.start_send(message) {
                    return Poll::Ready(Err(e));
                }
            }
        }

        // Flush
        {
            tracing::trace!("flushing");

            let mut dispatch = self.dispatch.lock();
            let sink = Pin::new(&mut dispatch.framed);
            Sink::<&M>::poll_flush(sink, cx)
        }
    }
}
