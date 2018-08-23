//! A chat server that broadcasts a message to all connections.
//!
//! This is a simple line-based server which accepts WebSocket connections,
//! reads lines from those connections, and broadcasts the lines to all other
//! connected clients.
//!
//! You can test this out by running:
//!
//!     cargo run --example server 127.0.0.1:12345
//!
//! And then in another window run:
//!
//!     cargo run --example client ws://127.0.0.1:12345/
//!
//! You can run the second command in multiple windows and then chat between the
//! two, seeing the messages from the other client as they're received. For all
//! connected clients they'll all join the same room and see everyone else's
//! messages.

extern crate futures;
extern crate tokio;
extern crate tokio_tungstenite;
extern crate tungstenite;

use std::env;
use std::io::{Error, ErrorKind};

use futures::stream::Stream;
use futures::Sink;
use futures::Future;
use tokio::net::TcpListener;
use tungstenite::protocol::Message;

use tokio_tungstenite::accept_async;

fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse().unwrap();

    // Create the event loop and TCP listener we'll accept connections on.
    let socket = TcpListener::bind(&addr).unwrap();
    println!("Listening on: {}", addr);

    let srv = socket.incoming().for_each(move |stream| {

        let addr = stream.peer_addr().expect("connected streams should have a peer address");

        accept_async(stream).and_then(move |ws_stream| {
            println!("New WebSocket connection: {}", addr);

            // Let's split the WebSocket stream, so we can work with the
            // reading and writing halves separately.
            let (mut sink, stream) = ws_stream.split();

            let msg: &[u8] = b"4204204200000000e4ef6f9a59000000020000000000000000000000000000080808088f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10500e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100bfafd3f1cc800309cb931081257b4e7edb0f9ea400187c550fd9141447ea086a5324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf0000000000000000000000000000000000000000000000000000000000000000";
            let ws_msg = tungstenite::Message::binary(msg);

            let ws_writer = sink.start_send(ws_msg);

            let ws_reader = stream.for_each(move |message: Message| {
                println!("Received a message from {}: {}", addr, message);

                Ok(())
            });

            // Now that we've got futures representing each half of the socket, we
            // use the `select` combinator to wait for either half to be done to
            // tear down the other. Then we spawn off the result.
            let connection = ws_reader.join(ws_writer);

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

    // Execute server.
    tokio::runtime::run(srv.map_err(|_e| ()));
}
