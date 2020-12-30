use std::{collections::HashMap, io::Cursor, sync::Arc};

use futures::{
    channel::{mpsc, oneshot},
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf},
    lock::Mutex as AsyncMutex,
    task::{Context, Poll},
    FutureExt, SinkExt, Stream, StreamExt,
};
use parking_lot::Mutex;

use beserial::SerializingError;
use nimiq_network_interface::message::{peek_type, read_message, Message};

/// # TODO
///
///  - Refactor to not use a spawn to handle the dispatching. Instead just add a poll method and poll it from the
///    handler.
///
pub struct MessageReceiver {
    channels: Arc<Mutex<HashMap<u64, Option<mpsc::Sender<Vec<u8>>>>>>,

    close_tx: Mutex<Option<oneshot::Sender<()>>>,

    error_rx: Mutex<oneshot::Receiver<SerializingError>>,
}

impl MessageReceiver {
    pub fn new<I: AsyncRead + Unpin + Send + Sync + 'static>(inbound: I) -> Self {
        let channels = Arc::new(Mutex::new(HashMap::new()));
        let (close_tx, close_rx) = oneshot::channel();
        let (error_tx, error_rx) = oneshot::channel();

        async_std::task::spawn({
            let channels = Arc::clone(&channels);

            async move {
                if let Err(e) = Self::reader(inbound, close_rx, channels).await {
                    log::warn!("Peer::reader: error: {}", e);
                    error_tx.send(e).unwrap();
                }
            }
        });

        Self {
            channels,
            close_tx: Mutex::new(Some(close_tx)),
            error_rx: Mutex::new(error_rx),
        }
    }

    pub(crate) fn poll_error(&self, cx: &mut Context) -> Poll<Option<SerializingError>> {
        match self.error_rx.lock().poll_unpin(cx) {
            Poll::Ready(Ok(e)) => Poll::Ready(Some(e)),
            Poll::Ready(Err(_)) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    pub fn close(&self) {
        if let Some(close_tx) = self.close_tx.lock().take() {
            // TODO: handle error
            close_tx.send(()).unwrap();
        } else {
            log::error!("MessageReceiver::close: Already closed.");
        }
    }

    /// Registers a receiver stream for a message type
    pub fn receive<M: Message>(&self) -> impl Stream<Item = M> {
        let (tx, rx) = mpsc::channel(16);

        let mut channels = self.channels.lock();
        if channels.contains_key(&M::TYPE_ID) {
            panic!("Receiver for type {} already registered.", M::TYPE_ID);
        }
        channels.insert(M::TYPE_ID, Some(tx));

        rx.filter_map(|buf| async move {
            let message = M::deserialize_message(&mut Cursor::new(buf))
                .map_err(|e| log::error!("MessageReceiver error: {}", e))
                .ok()?;

            Some(message)
        })
    }

    async fn reader<I: AsyncRead + Unpin + Send + Sync + 'static>(
        mut inbound: I,
        close_rx: oneshot::Receiver<()>,
        channels: Arc<Mutex<HashMap<u64, Option<mpsc::Sender<Vec<u8>>>>>>,
    ) -> Result<(), SerializingError> {
        let mut close_rx = close_rx.fuse();

        loop {
            log::trace!("Waiting for incoming message...");
            let data = futures::select! {
                result = read_message(&mut inbound).fuse() => {
                    log::trace!("read_message = {:?}", result);
                    result?
                },
                _ = close_rx => {
                    log::trace!("Received close event");
                    break;
                },
            };

            let message_type = peek_type(&data)?;

            log::trace!("Receiving message: type={}", message_type);
            log::trace!("Raw: {:?}", data);

            let mut tx = {
                let mut channels = channels.lock();

                if let Some(tx_opt) = channels.get_mut(&message_type) {
                    // This tx should be Some(_) because only this function takes out senders.
                    tx_opt.take().expect("Expected sender")
                } else {
                    log::warn!("No receiver for message type: {}", message_type);
                    // No receiver for this message type
                    continue;
                }
            };

            // Send data to receiver.
            if let Err(e) = tx.send(data).await {
                // An error occured, remove channel.
                log::error!("Error while receiving data: {}", e);

                channels.lock().remove(&message_type);
            } else {
                // Put tx back into Option
                *channels.lock().get_mut(&message_type).unwrap() = Some(tx);
            }
        }

        log::trace!("Message dispatcher task terminated");

        // Remove all channels (i.e. senders, which closes readers too?
        channels.lock().clear();

        Ok(())
    }
}

pub struct MessageSender<O>
where
    O: AsyncWrite,
{
    outbound: AsyncMutex<Option<O>>,
}

impl<O> MessageSender<O>
where
    O: AsyncWrite + Unpin,
{
    pub fn new(outbound: O) -> Self {
        Self {
            outbound: AsyncMutex::new(Some(outbound)),
        }
    }

    pub async fn close(&self) {
        log::trace!("Waiting for outbound lock...");
        let mut outbound = self.outbound.lock().await;

        if let Some(mut outbound) = outbound.take() {
            log::trace!("Closing outbound...");
            outbound.close().await.unwrap()
        } else {
            log::error!("Outbound already closed.");
        }
    }

    pub async fn send<M: Message>(&self, message: &M) -> Result<(), SerializingError> {
        let mut serialized = Vec::with_capacity(message.serialized_message_size());

        message.serialize_message(&mut serialized)?;

        log::trace!("Sending message: {:?}", serialized);

        let mut outbound = self.outbound.lock().await;

        if let Some(outbound) = outbound.as_mut() {
            outbound.write_all(&serialized).await?;
            Ok(())
        } else {
            log::error!("Outbound already closed.");
            Err(SerializingError::IoError(std::io::Error::from(std::io::ErrorKind::NotConnected)))
        }
    }
}

pub struct MessageDispatch<C>
where
    C: AsyncRead + AsyncWriteExt + Send + Sync,
{
    pub inbound: MessageReceiver,
    pub outbound: MessageSender<WriteHalf<C>>,
}

impl<C> MessageDispatch<C>
where
    C: AsyncRead + AsyncWriteExt + Send + Sync + 'static,
{
    pub fn new(socket: C) -> Self {
        let (reader, writer) = socket.split();

        Self {
            inbound: MessageReceiver::new(reader),
            outbound: MessageSender::new(writer),
        }
    }

    pub async fn close(&self) {
        log::trace!("MessageDispatch::close: Closing outbound");
        self.outbound.close().await;

        log::trace!("MessageDispatch::close: Closing inbound");
        self.inbound.close();

        log::trace!("MessageDispatch::close: Closed.");
    }
}
