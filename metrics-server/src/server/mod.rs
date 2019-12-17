use std::fmt::Display;
use std::io;
use std::sync::Arc;
use std::pin::Pin;
use std::task::{Poll, Context};
use std::future::Future;

use futures::{future, stream};
use futures::stream::StreamExt;
use hyper::{Body, Request, Response, StatusCode};
use hyper::body::Bytes;
use hyper::service::Service;
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE, LOCATION};
use base64::encode;

use crate::server::attributes::{CachedAttributes, VecAttributes};

pub mod attributes;

pub type SerializationType = Vec<u8>;

pub struct MetricsSerializer<W: io::Write + Into<Bytes>> {
    common_attributes: CachedAttributes,
    writer: W,
}

impl<W: io::Write + Into<Bytes>> MetricsSerializer<W> {
    #[inline]
    pub fn new<A: Into<CachedAttributes>>(common_attributes: A, writer: W) -> Self {
        MetricsSerializer {
            common_attributes: common_attributes.into(),
            writer,
        }
    }

    #[inline]
    pub fn metric<K: Display, V: Display>(&mut self, key: K, value: V) -> Result<(), io::Error> {
        writeln!(self.writer, "{}{{{}}} {}", key, self.common_attributes, value)
    }

    #[inline]
    pub fn metric_with_attributes<K: Display, V: Display, A: Into<VecAttributes>>(&mut self, key: K, value: V, attributes: A) -> Result<(), io::Error> {
        writeln!(self.writer, "{}{{{}}} {}", key, &self.common_attributes + attributes.into(), value)
    }
}

impl<W: io::Write + Into<Bytes>> From<MetricsSerializer<W>> for Bytes {
    fn from(serializer: MetricsSerializer<W>) -> Self {
        serializer.writer.into()
    }
}

pub trait Metrics: Send + Sync {
    fn metrics(&self, serializer: &mut MetricsSerializer<SerializationType>) -> Result<(), io::Error>;
}

#[derive(Debug)]
pub enum Never {}

impl std::error::Error for Never {
    fn description(&self) -> &str {
        match *self {}
    }
}

impl std::fmt::Display for Never {
    fn fmt(&self, _: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {}
    }
}

pub struct MetricsServer {
    metrics: Vec<Arc<dyn Metrics>>,
    common_attributes: CachedAttributes,
    username: Option<String>,
    password: Option<String>,
}

impl MetricsServer {
    #[inline]
    pub fn new<A: Into<CachedAttributes>>(metrics: Vec<Arc<dyn Metrics>>, common_attributes: A, username: Option<String>, password: Option<String>) -> Self {
        MetricsServer {
            metrics,
            common_attributes: common_attributes.into(),
            username,
            password,
        }
    }

    pub fn serve(&self) -> Body {
        let metrics = self.metrics.clone();
        let attributes = self.common_attributes.clone();
        let stream = stream::iter(metrics)
            .map(move |metrics| {
                let mut serializer = MetricsSerializer::new(attributes.clone(), Vec::new());
                match metrics.metrics(&mut serializer) {
                    Ok(()) => Ok(Bytes::from(serializer)),
                    Err(e) => Err(e),
                }

            });

        Body::wrap_stream(stream)
    }
}

fn check_auth(req: &Request<Body>, username: &Option<String>, password: &Option<String>) -> bool {
    match (username, password, req.headers().get(AUTHORIZATION).and_then(|header| header.to_str().ok())) {
        (None, None, _) => true,
        (Some(ref username), Some(ref password), Some(authorization)) => {
            authorization == format!("Basic {}", encode(&format!("{}:{}", username, password)))
        },
        _ => false,
    }
}

impl Service<Request<Body>> for MetricsServer {
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output=Result<Response<Body>, hyper::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> <Self as Service<Request<Body>>>::Future {
        // Check URI.
        let response = if req.uri() != "/metrics" {
            Response::builder()
                .status(StatusCode::MOVED_PERMANENTLY)
                .header(LOCATION, "/metrics")
                .body(Body::empty())
                .unwrap()
        }
        // Check authentication.
        else if !check_auth(&req, &self.username, &self.password) {
            Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(WWW_AUTHENTICATE, "Basic realm=\"Use username metrics and user-defined password to access metrics.\" charset=\"UTF-8\"")
                .body(Body::empty())
                .unwrap()
        }
        else {
            Response::new(self.serve())
        };

        Box::pin(future::ok(response))
    }
}
