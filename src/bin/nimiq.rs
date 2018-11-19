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
use nimiq::network::websocket::{SharedNimiqMessageStream, NimiqMessageStreamError};
use std::sync::Arc;
use parking_lot::Mutex;
use nimiq::network::connection::network_connection::CloseFuture;
use nimiq::network::connection::close_type::CloseType;
use futures::sync::mpsc::unbounded;
use futures::sync::oneshot;

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
        let shared_stream: SharedNimiqMessageStream = msg_stream.into();

        let (closing_tx, closing_rx) = oneshot::channel();
        let closing_tx = Arc::new(Mutex::new(Some(closing_tx)));

        let (tx, rx) = unbounded();
        let ws_writer = rx.forward(shared_stream.clone());

        let mut conntx = tx.clone();
        let counter = Arc::new(Mutex::new(0usize));
        let inner_counter = counter.clone();
        let ws_reader = shared_stream.clone().for_each(move |msg| {
            let mut counter = inner_counter.lock();
            *counter += 1;
            if *counter == 2 {
                println!("Closed!");
                let tx = closing_tx.lock().take().unwrap();
                tx.send(());
                return Ok(());
            }
            println!("Got message type: {:?}", msg.ty());
            conntx.unbounded_send(msg).map_err(|_| NimiqMessageStreamError::TagMismatch)?;
            Ok(())
        });

        let connection = (ws_reader.map_err(|_| ())
            .select(ws_writer.map(|_| ()).map_err(|_| ()))).map(|_| ()).map_err(|_| ())
            .select(closing_rx.map(|_: ()| ()).map_err(|_| ()))
            .then(move |_| CloseFuture::new(shared_stream.clone(), CloseType::AbortedSync))
            .map(|_: ()| ()).map_err(|_| ());

        tokio::spawn(connection.then(move |_| {
            Ok(())
        }));
        Ok(())
    });

    // Create a future that will resolve once both client and server futures resolve
    // (and since the server part will never resolve, this will run forever)
    let both = client;//.join(srv);

    // Execute both setups in Tokio
    tokio::run(both.map(|_| ()).map_err(|_| ()));
}
