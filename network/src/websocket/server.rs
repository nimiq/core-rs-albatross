use tokio::net::TcpStream;
use tokio_tungstenite::{accept_hdr_async, MaybeTlsStream};
use tungstenite::handshake::server::Callback;

use network_primitives::address::net_address::NetAddress;

use crate::websocket::error::Error;
use crate::websocket::NimiqMessageStream;

/// Accept an incoming connection and return a Future that will resolve to a NimiqMessageStream
pub async fn nimiq_accept_async<C>(stream: MaybeTlsStream<TcpStream>, net_address: NetAddress, callback: C) -> Result<NimiqMessageStream, Error>
    where C: Callback + Unpin,
{
    match accept_hdr_async(stream, callback).await {
        Ok(ws_stream) => Ok(NimiqMessageStream::new(ws_stream, net_address, false)),
        Err(e) => Err(e.into()),
    }
}
