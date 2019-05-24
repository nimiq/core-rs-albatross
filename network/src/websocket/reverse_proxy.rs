use std::str::from_utf8;
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::Mutex;
use reqwest::StatusCode;
use tungstenite::handshake::server::{Callback, ErrorResponse, Request};

use network_primitives::address::NetAddress;

use crate::network_config::ReverseProxyConfig;

/// Struct that stores relevant data for setting up reverse proxy support.
#[derive(Debug)]
pub struct ReverseProxyCallback {
    reverse_proxy_config: Option<ReverseProxyConfig>,
    remote_address: Mutex<Option<NetAddress>>,
}

impl ReverseProxyCallback {
    pub fn new(reverse_proxy_config: Option<ReverseProxyConfig>) -> Arc<Self> {
        Arc::new(ReverseProxyCallback {
            reverse_proxy_config,
            remote_address: Mutex::new(None),
        })
    }

    /// Returns the net address found in the header.
    pub fn header_net_address(&self) -> Option<NetAddress> {
        *self.remote_address.lock()
    }

    /// This function takes the net address given by the stream
    /// and checks it against the reverse proxy configuration.
    /// The function returns the correct net address of the peer
    /// after successful verification (see below).
    /// Otherwise it returns None.
    ///
    /// Verification steps:
    /// 1) Check whether a reverse proxy was configured.
    ///   - If so, continue,
    ///   - Else, return net address given by the stream.
    /// 2) Check whether the stream net address equals to the configured reverse proxy address.
    ///   - Return None on failure (except if config could not be parsed correctly, then display warning).
    /// 3) Check whether there was a header present with the real peer's net address.
    ///   - Return this if it was found, None otherwise.
    pub fn check_reverse_proxy(&self, stream_net_address: NetAddress) -> Option<NetAddress> {
        if let Some(ref config) = self.reverse_proxy_config {
            let proxy_net_address = &config.address;
            if proxy_net_address != &stream_net_address {
                error!("Received connection from {} when all connections were expected from the reverse proxy at {}: closing the connection", stream_net_address, proxy_net_address);
                return None;
            }

            // Return the address from the header.
            return self.header_net_address();
        }
        Some(stream_net_address)
    }
}

impl<'a> Callback for &'a ReverseProxyCallback {
    fn on_request(self, request: &Request) -> Result<Option<Vec<(String, String)>>, ErrorResponse> {
        if let Some(ref config) = self.reverse_proxy_config {
            if let Some(value) = request.headers.find_first(&config.header) {
                let str_value = from_utf8(value).map_err(|_| ErrorResponse {
                    error_code: StatusCode::INTERNAL_SERVER_ERROR,
                    headers: None,
                    body: None,
                })?;
                let str_value = str_value.split(',').next().ok_or(ErrorResponse {
                    error_code: StatusCode::INTERNAL_SERVER_ERROR,
                    headers: None,
                    body: None,
                })?; // Take first value from list.
                let str_value = str_value.trim();
                let net_address = NetAddress::from_str(str_value)
                    .map_err(|_|
                         ErrorResponse {
                             error_code: StatusCode::INTERNAL_SERVER_ERROR,
                             headers: None,
                             body: Some("Expected header to contain the real IP from the connecting client: closing the connection".to_string()),
                         }
                    )?;
                self.remote_address.lock().replace(net_address);
            } else {
                return Err(ErrorResponse {
                        error_code: StatusCode::INTERNAL_SERVER_ERROR,
                        headers: None,
                        body: Some("Expected header to contain the real IP from the connecting client: closing the connection".to_string()),
                    });
            }
        }
        Ok(None)
    }
}

pub trait ToCallback<C: Callback> {
    fn to_callback(self) -> C;
}

/// Wrapper required since we cannot implement Callback for Arc<ReverseProxyCallback>.
#[derive(Clone, Debug)]
pub struct ShareableReverseProxyCallback(Arc<ReverseProxyCallback>);

impl Callback for ShareableReverseProxyCallback {
    fn on_request(self, request: &Request) -> Result<Option<Vec<(String, String)>>, ErrorResponse> {
        self.0.as_ref().on_request(request)
    }
}

impl From<Arc<ReverseProxyCallback>> for ShareableReverseProxyCallback {
    fn from(arc: Arc<ReverseProxyCallback>) -> Self {
        ShareableReverseProxyCallback(arc)
    }
}

impl ToCallback<ShareableReverseProxyCallback> for Arc<ReverseProxyCallback> {
    fn to_callback(self) -> ShareableReverseProxyCallback {
        ShareableReverseProxyCallback::from(self)
    }
}