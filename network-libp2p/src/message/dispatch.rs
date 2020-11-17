use std::{
    collections::HashMap,
    sync::Arc,
    future::Future,
    pin::Pin,
    io::Cursor,
};

use futures::{
    channel::mpsc,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf},
    task::{Context, Poll},
    lock::Mutex as AsyncMutex,
    pin_mut, SinkExt, StreamExt, Stream, FutureExt,
};
use libp2p::swarm::NegotiatedSubstream;
use parking_lot::Mutex;
use pin_project::pin_project;

use beserial::SerializingError;
use nimiq_network_interface::message::{Message, read_message, peek_type};

use super::peer::Peer;


pub struct MessageReceiver {
    channels: Arc<Mutex<HashMap<u64, Option<mpsc::Sender<Vec<u8>>>>>>,
}


impl MessageReceiver
{
    pub fn new<I: AsyncRead + Send + Sync + 'static>(inbound: I) -> Self {
        let channels = Arc::new(Mutex::new(HashMap::new()));

        // FIXME: Poll this from the network behaviour
        async_std::task::spawn(Self::reader(inbound, Arc::clone(&channels)));

        Self {
            channels,
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

    async fn reader<I: AsyncRead + Send + Sync + 'static>(mut inbound: I, channels: Arc<Mutex<HashMap<u64, Option<mpsc::Sender<Vec<u8>>>>>>) -> Result<(), SerializingError> {
        pin_mut!(inbound);

        loop {
            let data = read_message(&mut inbound).await?;
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
}
