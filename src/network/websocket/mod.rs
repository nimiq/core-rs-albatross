use std::{io, fmt, net, time::Instant, fmt::Debug};

use url::Url;
use futures::prelude::*;
use tokio::{
    net::TcpStream,
};

use beserial::{Deserialize, Serialize};
use byteorder::{BigEndian, ByteOrder};

use tungstenite::{
    protocol::Message as WebSocketMessage,
    error::Error as WsError
};
use tokio_tungstenite::{
    MaybeTlsStream,
    WebSocketStream,
    connect_async,
    accept_async,
    stream::PeerAddr
};

use crate::utils::locking::MultiLock;
use crate::network::{
    address::net_address::NetAddress,
    message::Message as NimiqMessage
};

pub mod web_socket_connector;

type WebSocketLayer = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub trait IntoData {
    fn into_data(self) -> Vec<u8>;
}

impl IntoData for WebSocketMessage {
    fn into_data(self) -> Vec<u8> {
        self.into_data()
    }
}

#[derive(Debug)]
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
    net_address: NetAddress,
    outbound: bool,
    last_chunk_received_at: Option<Instant>,
}

impl NimiqMessageStream
{
    fn new(ws_socket: WebSocketStream<MaybeTlsStream<TcpStream>>, outbound: bool) -> Self {
        let peer_addr = ws_socket.get_ref().peer_addr().unwrap();
        return NimiqMessageStream {
            inner: ws_socket,
            processing_tag: 0,
            sending_tag: 0,
            buf: Vec::with_capacity(64), // 1/10th of the max number of messages we would ever need to store
            net_address: match peer_addr.ip() {
                net::IpAddr::V4(ip4) => NetAddress::IPv4(ip4),
                net::IpAddr::V6(ip6) => NetAddress::IPv6(ip6),
            },
            outbound: outbound,
            last_chunk_received_at: None,
        };
    }

    pub fn net_address(&self) -> &NetAddress {
        &self.net_address
    }

    pub fn outbound(&self) -> bool {
        self.outbound
    }

    pub fn last_chunk_received_at(&self) -> &Option<Instant> {
        &self.last_chunk_received_at
    }
}

impl Sink for NimiqMessageStream
{
    type SinkItem = NimiqMessage;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        // Save and increment tag.
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
                    // FIXME If this happens, we will try sending the whole message again with a new tag.
                    // This should be improved, e.g. using https://docs.rs/futures/0.2.1/futures/sink/struct.Buffer.html.
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
            Ok(r#async) => Ok(r#async),
            Err(error) => Err(()),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.close() {
            Ok(r#async) => Ok(r#async),
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

        // Update last chunk timestamp
        self.last_chunk_received_at = Some(Instant::now());

        // We have enough ws messages to create a nimiq message
        // FIXME: validate formula
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

            assert_eq!(remaining_length, 0, "Data missing");

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

impl Debug for NimiqMessageStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "NimiqMessageStream {{}}")
    }
}

/// Connect to a given URL and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_connect_async(url: Url) -> Box<Future<Item = NimiqMessageStream, Error = io::Error> + Send>
{
    Box::new(connect_async(url).map(|(ws_stream,_)| NimiqMessageStream::new(ws_stream, true))
    .map_err(|e| {
        println!("Error while trying to connect to another node: {}", e);
        io::Error::new(io::ErrorKind::Other, e)
    }))
}

/// Accept an incoming connection and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_accept_async(stream: MaybeTlsStream<TcpStream>) -> Box<Future<Item = NimiqMessageStream, Error = io::Error> + Send>
{
    Box::new(accept_async(stream).map(|ws_stream| NimiqMessageStream::new(ws_stream, false))
    .map_err(|e| {
        println!("Error while accepting a connection from another node: {}", e);
        io::Error::new(io::ErrorKind::Other, e)
    }))
}

#[derive(Debug)]
pub struct SharedNimiqMessageStream {
    inner: MultiLock<NimiqMessageStream>,
    net_address: NetAddress,
    outbound: bool,
    last_chunk_received_at: Option<Instant>,
}

impl SharedNimiqMessageStream {
    pub fn net_address(&self) -> NetAddress {
        self.net_address.clone()
    }

    pub fn outbound(&self) -> bool {
        self.outbound
    }

    pub fn last_chunk_received_at(&self) -> Option<&Instant> {
        self.last_chunk_received_at.as_ref()
    }
}

impl From<NimiqMessageStream> for SharedNimiqMessageStream {
    fn from(stream: NimiqMessageStream) -> Self {
        let net_address = stream.net_address().clone();
        let outbound = stream.outbound();
        let last_chunk_received_at = stream.last_chunk_received_at().clone();
        SharedNimiqMessageStream {
            net_address,
            outbound,
            last_chunk_received_at,
            inner: MultiLock::new(stream),
        }
    }
}

impl Clone for SharedNimiqMessageStream {
    #[inline]
    fn clone(&self) -> Self {
        SharedNimiqMessageStream {
            net_address: self.net_address.clone(),
            outbound: self.outbound,
            last_chunk_received_at: self.last_chunk_received_at.clone(),
            inner: self.inner.clone()
        }
    }
}

impl Stream for SharedNimiqMessageStream {
    type Item = NimiqMessage;
    type Error = NimiqMessageStreamError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                self.last_chunk_received_at = inner.last_chunk_received_at().clone();
                inner.poll()
            },
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
