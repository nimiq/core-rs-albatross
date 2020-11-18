use std::{
    collections::HashMap,
    sync::Arc,
    io::Cursor,
};

use futures::{
    channel::{mpsc, oneshot},
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf},
    lock::Mutex as AsyncMutex,
    task::{Context, Poll},
    pin_mut, SinkExt, StreamExt, Stream, FutureExt,
};
use parking_lot::Mutex;

use beserial::SerializingError;
use nimiq_network_interface::message::{Message, read_message, peek_type};


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


impl MessageReceiver
{
    pub fn new<I: AsyncRead + Send + Sync + 'static>(inbound: I) -> Self {
        let channels = Arc::new(Mutex::new(HashMap::new()));
        let (close_tx, close_rx) = oneshot::channel();
        let (error_tx, error_rx) = oneshot::channel();

        async_std::task::spawn({
            let channels = Arc::clone(&channels);

            async move {
                if let Err(e) = Self::reader(inbound, close_rx, channels).await {
                    log::error!("Peer::reader: error: {}", e);
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

    pub async fn close(&self) {
        if let Some(close_tx) = self.close_tx.lock().take() {
            // TODO: handle error
            close_tx.send(()).unwrap();

            // Remove all channels (i.e. senders, which closes readers too?
            self.channels.lock().clear();
        }
    }

    /// Registers a receiver stream for a message type
    pub fn receive<M: Message>(&self) -> impl Stream<Item=M> {
        let (tx, rx) = mpsc::channel(16);

        let mut channels = self.channels.lock();
        if channels.contains_key(&M::TYPE_ID) {
            panic!("Receiver for type {} already registered.", M::TYPE_ID);
        }
        channels.insert(M::TYPE_ID, Some(tx));

        rx.filter_map(|buf| {
            async move {
                let message = M::deserialize_message(&mut Cursor::new(buf))
                    .map_err(|e| log::error!("MessageReceiver error: {}", e))
                    .ok()?;

                Some(message)
            }
        })
    }

    async fn reader<I: AsyncRead + Send + Sync + 'static>(inbound: I, close_rx: oneshot::Receiver<()>, channels: Arc<Mutex<HashMap<u64, Option<mpsc::Sender<Vec<u8>>>>>>) -> Result<(), SerializingError> {
        pin_mut!(inbound);

        let mut close_rx = close_rx.fuse();

        loop {
            let data = futures::select! {
                data = read_message(&mut inbound).fuse() => data?,
                _ = close_rx => break,
            };

            let message_type = peek_type(&data)?;

            log::debug!("Receiving message: type={}", message_type);
            log::debug!("Raw: {:?}", data);

            let mut tx = {
                let mut channels = channels.lock();

                if let Some(tx_opt) = channels.get_mut(&message_type) {
                    // This tx should be Some(_) because only this function takes out senders.
                    tx_opt.take().expect("Expected sender")
                }
                else {
                    log::warn!("No receiver for message type: {}", message_type);
                    // No receiver for this message type
                    continue
                }
            };

            // Send data to receiver.
            if let Err(e) = tx.send(data).await {
                // An error occured, remove channel.
                log::error!("Error while receiving data: {}", e);

                channels.lock().remove(&message_type);
            }
            else {
                // Put tx back into Option
                *channels.lock().get_mut(&message_type).unwrap() = Some(tx);
            }
        }

        log::debug!("Message dispatcher task terminated");

        Ok(())
    }
}


pub struct MessageSender<O>
    where
        O: AsyncWrite,
{
    outbound: AsyncMutex<O>,
}

impl<O> MessageSender<O>
    where
        O: AsyncWrite + Unpin,
{
    pub fn new(outbound: O) -> Self {
        Self {
            outbound: AsyncMutex::new(outbound),
        }
    }

    pub async fn close(&self) {
        self.outbound.lock().await.close().await.unwrap();
    }

    pub async fn send<M: Message>(&self, message: &M) -> Result<(), SerializingError> {
        let mut serialized = Vec::with_capacity(message.serialized_message_size());

        message.serialize_message(&mut serialized)?;

        log::debug!("Sending message: {:?}", serialized);

        if let Err(e) = self.outbound.lock().await.write_all(&serialized).await {
            panic!("Sending message failed: {}", e);
        }
        else {
            Ok(())
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
        self.inbound.close().await;
        self.outbound.close().await;
    }
}
