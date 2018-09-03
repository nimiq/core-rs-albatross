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
use futures::sync::mpsc::channel;

fn main() {
    let test: ConnectAsync<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, io::Error> = nimiq_connect_async(url::Url::parse("ws://127.0.0.1:8080").unwrap());
    let test = test.and_then(|msg_stream| {
        let (sink, stream) = msg_stream.split();

        let (tx, rx) = channel(10);
        let ws_writer = rx.forward(sink);

        let mut conntx = tx.clone();
        let ws_reader = stream.for_each(move |msg| {
            println!("Got message type: {:?}", msg.ty());
            conntx.start_send(msg).expect("se escochero la vara");
            Ok(())
        });

        let connection = ws_reader.map_err(|_| ())
            .select(ws_writer.map(|_| ()).map_err(|_| ()));

        tokio::spawn(connection.then(move |_| {
            Ok(())
        }));
        Ok(())
    });

     tokio::run(test.map_err(|_e| ()));
}
