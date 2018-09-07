use beserial::{Deserialize, Serialize};
use byteorder::{BigEndian, ByteOrder};
use futures::prelude::*;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use network::message::Message as NimiqMessage;
use tungstenite::protocol::Message as WebSocketMessage;

use tungstenite::error::Error as WsError;
use url::Url;
use tokio::net::TcpStream;
use tokio_tungstenite::connect_async;
use std::io;
use std;
use utils::locking::MultiLock;

type WebSocketLayer = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub trait IntoData {
    fn into_data(self) -> Vec<u8>;
}

impl IntoData for WebSocketMessage {
    fn into_data(self) -> Vec<u8> {
        self.into_data()
    }
}

pub enum NimiqMessageStreamError {
    WebSocketError(WsError),
    TagMismatch,
}

const MAX_CHUNK_SIZE: usize = 1024 * 16; // 16 kb
pub struct NimiqMessageStream
{
    inner: WebSocketLayer,
    processing_tag: u8,
    sending_tag: u8,
    buf: Vec<WebSocketMessage>,
}

impl NimiqMessageStream
{
    fn new(ws_socket: WebSocketLayer) -> Self {
        return NimiqMessageStream {
            inner: ws_socket,
            processing_tag: 0,
            sending_tag: 0,
            buf: Vec::with_capacity(64), // 1/10th of the max number of messages we would ever need to store
        };
    }
}

impl Sink for NimiqMessageStream
{
    type SinkItem = NimiqMessage;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        // Save and increment tag. (TODO: what about locking?)
        let tag = self.sending_tag;
        self.sending_tag = self.sending_tag.wrapping_add(1);

        let msg = item.serialize_to_vec();

        // Send chunks to underlying layer.
        let mut remaining = msg.len();
        let mut chunk;
        while remaining > 0 {
            let mut buffer;
            if remaining + /*tag*/ 1 >= MAX_CHUNK_SIZE {
                buffer = Vec::with_capacity(MAX_CHUNK_SIZE + /*tag*/ 1);
                buffer.push(tag);
                chunk = &msg[msg.len() - remaining..MAX_CHUNK_SIZE - 1];
            } else {
                buffer = Vec::with_capacity(remaining + /*tag*/ 1);
                buffer.push(tag);
                chunk = &msg[msg.len() - remaining..remaining];
            }

            buffer.extend(chunk);

            match self.inner.start_send(WebSocketMessage::binary(buffer)) {
                Ok(state) => match state {
                    AsyncSink::Ready => (),
                    // We started to send some chunks, but now the queue is full:
                    AsyncSink::NotReady(_) => return Ok(AsyncSink::NotReady(item)),
                },
                Err(error) => return Err(()),
            };

            remaining -= chunk.len();
        }
        // We didn't exit previously, so everything worked out.
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_complete() {
            Ok(async) => Ok(async),
            Err(error) => Err(()),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.close() {
            Ok(async) => Ok(async),
            Err(error) => Err(()),
        }
    }
}

impl Stream for NimiqMessageStream
{
    type Item = NimiqMessage;
    type Error = NimiqMessageStreamError;

    // FIXME: This implementation is inefficient as it tries to construct the nimiq message
    // everytime, it should cache the work already done and just do new work on each iteration
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // println!("Starting `poll()`");

        // First, lets get as many WebSocket messages as available and store them in the buffer
        loop {
            match self.inner.poll() {
                Ok(Async::Ready(Some(m))) => self.buf.push(m),
                Ok(Async::Ready(None)) => return Ok(Async::Ready(None)), // FIXME: first flush our buffer and _then_ signal that there will be no more messages available
                Ok(Async::NotReady) => break,
                Err(e) => {
                    println!("Error condition: {:?}", e);
                    return Err(NimiqMessageStreamError::WebSocketError(e)) // FIXME: first flush our buffer and _then_ signal that there was an error
                    },
            }
        }
        // println!("made it after the polling of tungstenite");

        // If there are no web socket messages in the buffer, signal that we don't have anything yet
        // (i.e. we would need to block waiting, which is a no no in an async function)
        if self.buf.len() == 0 {
            println!("There are no ws msgs yet, poll later");
            return Ok(Async::NotReady);
        }

        // Now let's try to process the web socket messages that we have in order to create
        // a nimiq message
        // FIXME: DRY: this should be integrated into the code in the loop below
        let mut ws_message = self.buf.remove(0).into_data();

        // Make sure the tag is the one we expect
        let foo = ws_message.remove(0);
        println!("tag: {}, foo.tag: {}", self.processing_tag, foo);
        if self.processing_tag != foo {
            println!("Tag mismatch!");
            return Err(NimiqMessageStreamError::TagMismatch);
        }

        // look at length, if we don't have enough ws msgs to create a nimiq msg, return Ok(Async::NotReady)
        // if we have enough, check their tags and if all of them match, remove them from buf and process them.
        // FIXME: what happens if one or more of the tags don't match?

        // Get the length of this message
        // FIXME: support for message types > 253 is pending (it changes the length position in the chunk)
        // The magic number is 4 bytes and the type is 1 byte, so we want to start at the 6th byte (index 5), and the length field is 4 bytes
        let msg_length = &ws_message[5..9];
        let msg_length = BigEndian::read_u32(msg_length) as usize;

        // We have enough ws messages to create a nimiq message
        if msg_length < ((1 + self.buf.len()) * (MAX_CHUNK_SIZE + 1)) {
            // println!("We have enough ws msgs to create a Nimiq msg!");

            let mut binary_data = ws_message.clone(); // FIXME: clone is slow, fix the problem by figuring out how to do line 94 & 95 without borrow
            let mut remaining_length = msg_length - binary_data.len();

            // Get more ws messages until we have all the ones that compose this nimiq message
            while remaining_length > 0 {
                let mut ws_message = self.buf.remove(0).into_data(); // FIXME: slow, better to count how many are needed and remove them all at once

                // If the tag is correct, then append the data to our buffer
                let current_message_length: usize;
                if self.processing_tag == ws_message.remove(0) {
                    current_message_length = ws_message.len();
                    binary_data.append(&mut ws_message);
                } else {
                    return Err(NimiqMessageStreamError::TagMismatch);
                }

                remaining_length -= current_message_length;
            }

            // At this point we already read all the messages we need into the binary_data variable
            let nimiq_message: NimiqMessage =
                Deserialize::deserialize(&mut &binary_data[..]).unwrap();
            self.processing_tag += 1;
            return Ok(Async::Ready(Some(nimiq_message)));
        } else {
            println!("We don't have enough ws msgs yet, poll again later");
            return Ok(Async::NotReady);
        }
    }
}

/// Future returned from nimiq_connect_async() which will resolve
/// once the tokio-tungstenite connection is established.
pub struct ConnectAsync<S: Stream + Sink, E>
    where S::Item: IntoData
{
    inner: Box<Future<Item = S, Error = E> + Send>,
}

impl<E> Future for ConnectAsync<WebSocketLayer, E>
{
    type Item = NimiqMessageStream;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(ws) => Ok(Async::Ready(NimiqMessageStream::new(ws))),
        }
    }
}

/// Connect to a given URL.
pub fn nimiq_connect_async(url: Url)
                           -> ConnectAsync<WebSocketStream<MaybeTlsStream<TcpStream>>, io::Error>
{
    let connect = Box::new(
        connect_async(url).map(|(ws,_)| ws).map_err(|e| {
            println!("Error during the websocket handshake occurred: {}", e);
            io::Error::new(io::ErrorKind::Other, e)
        })
    );

    ConnectAsync {inner: connect}
}

pub struct SharedNimiqMessageStream {
    inner: MultiLock<NimiqMessageStream>,
}

impl From<NimiqMessageStream> for SharedNimiqMessageStream {
    fn from(stream: NimiqMessageStream) -> Self {
        SharedNimiqMessageStream {
            inner: MultiLock::new(stream),
        }
    }
}

impl Clone for SharedNimiqMessageStream {
    #[inline]
    fn clone(&self) -> Self {
        SharedNimiqMessageStream {
            inner: self.inner.clone()
        }
    }
}

impl Stream for SharedNimiqMessageStream {
    type Item = NimiqMessage;
    type Error = NimiqMessageStreamError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.poll(),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl Sink for SharedNimiqMessageStream {
    type SinkItem = NimiqMessage;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem)
                  -> StartSend<Self::SinkItem, Self::SinkError>
    {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.start_send(item),
            Async::NotReady => Ok(AsyncSink::NotReady(item)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.poll_complete(),
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.close(),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
