use std::net::IpAddr;

use network_primitives::address::net_address::NetAddress;
use tokio::net::TcpStream;
use tokio_tungstenite::{client_async_tls, MaybeTlsStream, WebSocketStream};
use tungstenite::handshake::client::{Request, Response};
use url::Url;

use crate::websocket::error::Error;
use crate::websocket::NimiqMessageStream;

/// Connect to a given URL and return a Future that will resolve to a NimiqMessageStream
pub async fn nimiq_connect_async(url: Url) -> Result<NimiqMessageStream, Error> {
    match connect_async(url).await {
        Ok((ws_stream, _, net_address)) => {
            Ok(NimiqMessageStream::new(ws_stream, net_address, true))
        },
        Err(e) => Err(e.into()),
    }
}

/// Connect to a given URL.
pub async fn connect_async<R>(
    request: R,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response, NetAddress), tungstenite::Error>
    where
        R: Into<Request<'static>> + Unpin,
{
    let request: Request = request.into();

    let domain =  match request.url.host_str() {
        Some(d) => Ok(d.to_string()),
        None => Err(tungstenite::Error::Url("no host name in the url".into())),
    }?;
    let port = request
        .url
        .port_or_known_default()
        .expect("Bug: port unknown");

    let addr = format!("{}:{}", domain, port);
    let try_socket = TcpStream::connect(addr).await;
    let socket = try_socket.map_err(tungstenite::Error::Io)?;
    let peer_address = socket.peer_addr()
        .map_err(tungstenite::Error::Io)?;
    let net_address = match peer_address.ip() {
        IpAddr::V4(addr) => NetAddress::IPv4(addr),
        IpAddr::V6(addr) => NetAddress::IPv6(addr),
    };
    let (stream, response) = client_async_tls(request, socket).await?;
    Ok((stream, response, net_address))
}
