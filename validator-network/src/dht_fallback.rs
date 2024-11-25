use std::{io, str::FromStr};

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
            io::Error::new(io::ErrorKind::InvalidInput, format!("invalid URI: {}", url))
        })?;
        Ok(DhtFallback { client, uri })
    }
    pub fn new(url: Url) -> Option<DhtFallback> {
        DhtFallback::new_inner(url)
            .inspect_err(|error| error!(%error, "couldn't create http client"))
            .ok()
    }

    async fn resolve_inner<T: FromStr>(
        &self,
        validator_address: Address,
    ) -> Result<Option<T>, String> {
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
    pub async fn resolve<T: FromStr>(&self, validator_address: Address) -> Option<T> {
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
                    "NQ26 0000 0000 02A5 YAK7 4QNF 9MH0 TE2B GVRU"
                        .parse()
                        .unwrap()
                )
                .await,
            Some(String::from(
                "12D3KooWD9bYVKHXm7H6RErXJLhYbPG6EN5g8wW3JqPWAi9ERwfo"
            ))
        );
    }
}
