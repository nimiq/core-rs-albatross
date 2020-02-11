use std::borrow::Cow;
use std::net::IpAddr;

use futures::StreamExt;
use network_primitives::address::net_address::NetAddress;
use tokio::net::TcpStream;
use tokio_tungstenite::{client_async_tls, MaybeTlsStream, WebSocketStream};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::client::Response;
use url::Url;

use crate::websocket::error::Error;
use crate::websocket::NimiqMessageStream;

/// Connect to a given URL and return a Future that will resolve to a NimiqMessageStream
pub async fn nimiq_connect_async(url: Url) -> Result<NimiqMessageStream, Error> {
    match connect_async(url).await {
        Ok((ws_stream, _, net_address)) => {
            let (tx, rx) = ws_stream.split();
            Ok(NimiqMessageStream::new(rx, tx, net_address, true))
        },
        Err(e) => Err(e.into()),
    }
}

/// Connect to a given URL.
pub async fn connect_async<R>(
    request: R,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response, NetAddress), tungstenite::Error>
    where
        R: IntoClientRequest + Unpin,
{
    let request = request.into_client_request()?;

    let domain =  match request.uri().host() {
        Some(d) => Ok(d.to_string()),
        None => Err(tungstenite::Error::Url("no host name in the url".into())),
    }?;
    let port = match request.uri().port_u16() {
        Some(p) => p,
        None => match request.uri().scheme_str() {
            Some("http") | Some("ws") => 80,
            Some("https") | Some("wss") => 443,
            _ => return Err(tungstenite::Error::Url(
                Cow::Owned(format!("Invalid port in url: {}", request.uri())))),
        }
    };

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
