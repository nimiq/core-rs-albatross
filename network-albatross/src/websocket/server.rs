use futures::future;
use futures::prelude::*;
use tokio::net::TcpStream;
use tokio_tungstenite::{accept_hdr_async, MaybeTlsStream};
use tungstenite::handshake::server::Callback;

use crate::websocket::error::Error;
use crate::websocket::NimiqMessageStream;

/// Accept an incoming connection and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_accept_async<C>(stream: MaybeTlsStream<TcpStream>, callback: C) -> Box<dyn Future<Item = NimiqMessageStream, Error = Error> + Send>
    where C: Callback + Send + 'static { // TODO: Why do we need 'static here?!
    Box::new(
        accept_hdr_async(stream, callback).then(|result| {
            match result {
                Ok(ws_stream) => future::result(NimiqMessageStream::new(ws_stream, false)),
                Err(e) => future::err(e.into())
            }
        })
    )
}