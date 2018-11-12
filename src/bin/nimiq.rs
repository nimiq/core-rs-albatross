extern crate url;
extern crate tokio;
extern crate nimiq;
extern crate futures;
extern crate beserial;
extern crate byteorder;
extern crate tungstenite;
extern crate tokio_tungstenite;

use futures::prelude::*;
use tokio::net::TcpListener;
use std::io::{Error, ErrorKind};

use nimiq::network::websocket::{nimiq_connect_async, nimiq_accept_async};
use futures::sync::mpsc::channel;

pub fn main() {
    let addr = "127.0.0.1:8081".to_string();
    let addr = addr.parse().unwrap();

    // Create the event loop and TCP listener we'll accept connections on.
    let socket = TcpListener::bind(&addr).unwrap();
    println!("Listening on: {}", addr);

    // Web Socket server setup

    let srv = socket.incoming().for_each(move |stream| {

        let addr = stream.peer_addr().expect("connected streams should have a peer address");

        nimiq_accept_async(stream).and_then(move |ws_stream| {
            println!("New WebSocket connection: {}", addr);

            // Let's split the WebSocket stream, so we can work with the
            // reading and writing halves separately.
            // let (sink, stream) = ws_stream.split();

            let ws_reader = ws_stream.for_each(move |msg| {
                println!("Got message type: {:?}", msg.ty());
                Ok(())
            });

            // Now that we've got futures representing each half of the socket, we
            // use the `select` combinator to wait for either half to be done to
            // tear down the other. Then we spawn off the result.
            let connection = ws_reader.map(|_| ()).map_err(|_| ());

            tokio::spawn(connection.then(move |_| {
                println!("Connection {} closed.", addr);
                Ok(())
            }));

            Ok(())
        }).map_err(|e| {
            println!("Error during the websocket handshake occurred: {}", e);
            Error::new(ErrorKind::Other, e)
        })
    });

    // Web Socket client setup
    let client = nimiq_connect_async(url::Url::parse("ws://127.0.0.1:8080").unwrap()).and_then(|msg_stream| {
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

    // Create a future that will resolve once both client and server futures resolve
    // (and since the server part will never resolve, this will run forever)
    let both = client.join(srv);

    // Execute both setups in Tokio
    tokio::run(both.map(|_| ()).map_err(|_| ()));
}
