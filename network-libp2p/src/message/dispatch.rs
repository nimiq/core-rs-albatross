use std::{
    collections::HashMap,
    sync::Arc,
    future::Future,
    pin::Pin,
};

use futures::{
    channel::mpsc,
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
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
    //future: Mutex<Box<dyn Future<Output=Result<(), SerializingError>> + Send + Sync + Unpin + 'static>>,
}

impl MessageReceiver
{
    pub fn new<I: AsyncRead + Send + Sync + 'static>(inbound: I) -> Self {
        let channels = Arc::new(Mutex::new(HashMap::new()));

        let future = Mutex::new(Box::new(Self::reader(inbound, Arc::clone(&channels))));
        //tokio::spawn(Self::reader(inbound, Arc::clone(&channels)));

        Self {
            channels,
            //future,
        }
    }

    pub(crate) fn poll(&self, cx: &mut Context) -> Poll<Result<(), SerializingError>> {
        unimplemented!();
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
                M::deserialize_from_vec(&buf)
                    .map_err(|e| log::error!("MessageReceiver error: {}", e))
                    .ok()
            }
        })
    }

    async fn reader<I: AsyncRead + Send + Sync + 'static>(mut inbound: I, channels: Arc<Mutex<HashMap<u64, Option<mpsc::Sender<Vec<u8>>>>>>) -> Result<(), SerializingError> {
        let mut inbound = inbound;

        pin_mut!(inbound);

        loop {
            let data = read_message(&mut inbound).await?;
            let message_type = peek_type(&data)?;

            let mut tx = if let Some(tx_opt) = channels.lock().get_mut(&message_type) {
                // This tx should be Some(_) because only this function takes out senders.
                tx_opt.take().expect("Expected sender")
            }
            else {
                // No receiver for this message type
                continue
            };

            // Send data to receiver.
            tx.send(data).await.unwrap(); // TODO: Handle error

            // Put tx back into Option
            *channels.lock().get_mut(&message_type).unwrap() = Some(tx);
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
        Ok(self.outbound.lock().await.write_all(&serialized).await?)
    }
}
