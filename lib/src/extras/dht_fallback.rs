use std::io;

use http::Uri;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper_rustls::{ConfigBuilderExt, HttpsConnector};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};
use log::error;
use nimiq_keys::Address;
use nimiq_network_libp2p::PeerId;
use serde::Deserialize;
use url::Url;

#[derive(Deserialize)]
struct Fallback {
    validators: Vec<FallbackValidator>,
}

#[derive(Deserialize)]
struct FallbackValidator {
    address: Address,
    peer_id: String,
}

pub struct DhtFallback {
    client: Client<HttpsConnector<HttpConnector>, Empty<Bytes>>,
    uri: Uri,
}

impl DhtFallback {
    fn new_inner(url: Url) -> io::Result<DhtFallback> {
        let tls = rustls::ClientConfig::builder()
            .with_native_roots()?
            .with_no_client_auth();

        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .build();

        let client = Client::builder(TokioExecutor::new()).build(https);
        let uri = url.as_str().parse().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("invalid URI: {url}"))
        })?;
        Ok(DhtFallback { client, uri })
    }
    pub fn new(url: Url) -> Option<DhtFallback> {
        DhtFallback::new_inner(url)
            .inspect_err(|error| error!(%error, "couldn't create http client"))
            .ok()
    }

    async fn resolve_inner(&self, validator_address: Address) -> Result<Option<PeerId>, String> {
        let response = self
            .client
            .get(self.uri.clone())
            .await
            .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!("bad http response: {}", response.status()));
        }

        let response = response
            .into_body()
            .collect()
            .await
            .map_err(|error| error.to_string())?
            .to_bytes();

        let fallback: Fallback =
            serde_json::from_slice(&response).map_err(|error| format!("invalid JSON: {error}"))?;

        for validator in fallback.validators {
            if validator.address == validator_address {
                return Ok(Some(validator.peer_id.parse().map_err(|_| {
                    format!("invalid peer ID: {:?}", validator.peer_id)
                })?));
            }
        }

        Ok(None)
    }
    pub async fn resolve(&self, validator_address: Address) -> Option<PeerId> {
        self.resolve_inner(validator_address.clone())
            .await
            .inspect_err(|error| error!(%error, %validator_address, "couldn't resolve"))
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod test {
    use url::Url;

    use super::DhtFallback;

    #[tokio::test]
    async fn resolve() {
        assert_eq!(
            DhtFallback::new(Url::parse("https://gist.githubusercontent.com/hrxi/50dc18caa17826e72cc05542cfe8946f/raw/dht.json").unwrap())
                .unwrap()
                .resolve(
                    "NQ36 U0BH 0BHM J0EH UAE5 FMV6 D2EY 8TBP 50M3"
                        .parse()
                        .unwrap()
                )
                .await,
            Some(
                "12D3KooW9tKu6QesTCCqADjhVSf4hvWinzjuLpxv4mefBzKLkiae".parse().unwrap())
        );
    }
}
