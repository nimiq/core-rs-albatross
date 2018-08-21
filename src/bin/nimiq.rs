// extern crate tokio;
// extern crate tokio_tls;
// extern crate url;

// #[macro_use]
// extern crate futures;

extern crate url;
// extern crate bytes;
extern crate beserial;
extern crate byteorder;
extern crate futures;
extern crate nimiq;
extern crate tokio;
extern crate tokio_tungstenite;
extern crate tungstenite;

// use tokio_tls::*;
// use tokio::io;
// use tokio::net::*;
// use tokio::prelude::*;

// use bytes::{BufMut, Bytes, BytesMut};
use beserial::Deserialize;
use byteorder::{BigEndian, ByteOrder};
use futures::prelude::*;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use nimiq::network::message::Message as NimiqMessage;
use tungstenite::protocol::Message as WebSocketMessage;

use tungstenite::error::Error as WsError;

pub trait IntoData {
    fn into_data(self) -> Vec<u8>;
}

impl IntoData for WebSocketMessage {
    fn into_data(self) -> Vec<u8> {
        self.into_data()
    }
}

pub enum NimiqMessageStreamError {
    WebSocketError,
    TagMismatch,
}

pub struct NimiqMessageStream<S: Stream + Sink>
where S::Item: IntoData
{
    ws_socket: S,
    processing_tag: u8,
    buf: Vec<S::Item>,
}

impl<S: Stream + Sink> NimiqMessageStream<S>
where S::Item: IntoData
{
    fn new(ws_socket: S) -> Self {
        return NimiqMessageStream {
            ws_socket,
            processing_tag: 0,
            buf: Vec::with_capacity(64), // 1/10th of the max number of messages we would ever need to store
        };
    }
}

impl<S: Stream + Sink> Stream for NimiqMessageStream<S>
where S::Item: IntoData
{
    type Item = NimiqMessage;
    type Error = NimiqMessageStreamError;

    // FIXME: This implementation is inefficient as it tries to construct the nimiq message
    // everytime, it should cache the work already done and just do new work on each iteration
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        const MAX_CHUNK_SIZE: usize = 1024 * 16; // 16 kb

        // First, lets get as many WebSocket messages as available and store them in the buffer
        loop {
            match self.ws_socket.poll() {
                Ok(Async::Ready(Some(m))) => self.buf.push(m),
                Ok(Async::Ready(None)) => return Ok(Async::Ready(None)), // FIXME: first flush our buffer and _then_ signal that there will be no more messages available
                Ok(Async::NotReady) => break,
                Err(_) => return Err(NimiqMessageStreamError::WebSocketError),
            }
        }

        // If there are no web socket messages in the buffer, signal that we don't have anything yet
        // (i.e. we would need to block waiting, which is a no no in an async function)
        if self.buf.len() == 0 {
            return Ok(Async::NotReady);
        }

        // Now let's try to process the web socket messages that we have in order to create
        // a nimiq message
        // FIXME: DRY: this should be integrated into the code in the loop below
        let mut ws_message = self.buf.remove(0).into_data();

        // Make sure the tag is the one we expect
        if self.processing_tag != ws_message.remove(0) {
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

impl<S: Stream + Sink, E> Future for ConnectAsync<S, E>
where S::Item: IntoData
{
    type Item = NimiqMessageStream<S>;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(ws) => Ok(Async::Ready(NimiqMessageStream::new(ws))),
        }
    }
}

/// Connect to a given URL.
pub fn nimiq_connect_async(url: url::Url)
    -> ConnectAsync<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, WsError>
{
    let connect = Box::new(tokio_tungstenite::connect_async(url).map(|(ws,_)| ws));

    ConnectAsync {inner: connect}
}

fn main() {
    let test: ConnectAsync<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, WsError> = nimiq_connect_async(url::Url::parse("ws://127.0.0.1:8080").unwrap());
    let test = test.and_then(|msg_stream| {
        let process_message = msg_stream.for_each(|_| {
            println!("Got message!");
            Ok(())
        });
        process_message.then(|_| Ok(()))
    });

     tokio::run(test.map(|_| ()).map_err(|_| ()));
}
