use futures::prelude::*;
use tokio_tungstenite::connect_async;
use url::Url;

use crate::websocket::error::Error;
use crate::websocket::NimiqMessageStream;

/// Connect to a given URL and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_connect_async(url: Url) -> Box<Future<Item = NimiqMessageStream, Error = Error> + Send> {
    Box::new(
    connect_async(url).map(|(ws_stream,_)| NimiqMessageStream::new(ws_stream, true))
            .map_err(move |e| {
                Error::WebSocketError(e)
            })
    )
}