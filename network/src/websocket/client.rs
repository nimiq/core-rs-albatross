use futures::future;
use futures::prelude::*;
use tokio_tungstenite::connect_async;
use url::Url;

use crate::websocket::error::Error;
use crate::websocket::NimiqMessageStream;

/// Connect to a given URL and return a Future that will resolve to a NimiqMessageStream
pub fn nimiq_connect_async(url: Url) -> Box<dyn Future<Item = NimiqMessageStream, Error = Error> + Send> {
    Box::new(
        connect_async(url).then(|result| {
            match result {
                Ok((ws_stream,_)) => future::result(NimiqMessageStream::new(ws_stream, true)),
                Err(e) => future::err(e.into())
            }
        })
    )
}