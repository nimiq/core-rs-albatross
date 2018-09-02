extern crate url;
extern crate tokio;
extern crate nimiq;
extern crate futures;
extern crate beserial;
extern crate byteorder;
extern crate tungstenite;
extern crate tokio_tungstenite;

use beserial::Deserialize;
use byteorder::{BigEndian, ByteOrder};
use futures::prelude::*;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use nimiq::network::message::Message as NimiqMessage;
use tungstenite::protocol::Message as WebSocketMessage;

use std::io;
use tungstenite::error::Error as WsError;
use nimiq::network::websocket::ConnectAsync;
use nimiq::network::websocket::nimiq_connect_async;

fn main() {
    let test: ConnectAsync<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, io::Error> = nimiq_connect_async(url::Url::parse("ws://127.0.0.1:8080").unwrap());
    let test = test.and_then(|msg_stream| {
        let process_message = msg_stream.for_each(|_| {
            println!("Got message!");
            Ok(())
        });
        process_message.then(|_| Ok(()))
    });

     tokio::run(test.map(|_| ()).map_err(|_| ()));
}
