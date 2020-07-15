use std::fmt::Display;
use std::io;
use std::sync::Arc;

use base64::encode;
use futures::IntoFuture;
use futures::{future, stream, stream::Stream, Future};
use hyper::header::{AUTHORIZATION, LOCATION, WWW_AUTHENTICATE};
use hyper::Chunk;
use hyper::{Body, Request, Response, StatusCode};

use crate::server::attributes::{CachedAttributes, VecAttributes};

pub mod attributes;

pub type SerializationType = Vec<u8>;

pub struct MetricsSerializer<W: io::Write + Into<Chunk>> {
    common_attributes: CachedAttributes,
    writer: W,
}

impl<W: io::Write + Into<Chunk>> MetricsSerializer<W> {
    #[inline]
    pub fn new<A: Into<CachedAttributes>>(common_attributes: A, writer: W) -> Self {
        MetricsSerializer {
            common_attributes: common_attributes.into(),
            writer,
        }
    }

    #[inline]
    pub fn metric<K: Display, V: Display>(&mut self, key: K, value: V) -> Result<(), io::Error> {
        writeln!(
            self.writer,
            "{}{{{}}} {}",
            key, self.common_attributes, value
        )
    }

    #[inline]
    pub fn metric_with_attributes<K: Display, V: Display, A: Into<VecAttributes>>(
        &mut self,
        key: K,
        value: V,
        attributes: A,
    ) -> Result<(), io::Error> {
        writeln!(
            self.writer,
            "{}{{{}}} {}",
            key,
            &self.common_attributes + attributes.into(),
            value
        )
    }
}

impl<W: io::Write + Into<Chunk>> From<MetricsSerializer<W>> for Chunk {
    fn from(serializer: MetricsSerializer<W>) -> Self {
        serializer.writer.into()
    }
}

pub trait Metrics: Send + Sync {
    fn metrics(
        &self,
        serializer: &mut MetricsSerializer<SerializationType>,
    ) -> Result<(), io::Error>;
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
    pub fn new<A: Into<CachedAttributes>>(
        metrics: Vec<Arc<dyn Metrics>>,
        common_attributes: A,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
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
        let stream = stream::iter_ok::<_, io::Error>(metrics).map(move |metrics| {
            let mut serializer = MetricsSerializer::new(attributes.clone(), Vec::new());
            match metrics.metrics(&mut serializer) {
                Ok(()) => Chunk::from(serializer),
                Err(e) => {
                    // TODO: Properly handle errors.
                    warn!("Metrics error: {}", e);
                    Chunk::default()
                }
            }
        });

        Body::wrap_stream(stream)
    }
}

fn check_auth(req: &Request<Body>, username: &Option<String>, password: &Option<String>) -> bool {
    match (
        username,
        password,
        req.headers()
            .get(AUTHORIZATION)
            .and_then(|header| header.to_str().ok()),
    ) {
        (None, None, _) => true,
        (Some(ref username), Some(ref password), Some(authorization)) => {
            authorization == format!("Basic {}", encode(&format!("{}:{}", username, password)))
        }
        _ => false,
    }
}

impl IntoFuture for MetricsServer {
    type Future = future::FutureResult<Self::Item, Self::Error>;
    type Item = Self;
    type Error = Never;

    fn into_future(self) -> Self::Future {
        future::ok(self)
    }
}

impl hyper::service::Service for MetricsServer {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = Box<dyn Future<Item = Response<Body>, Error = hyper::Error> + Send>;

    fn call(
        &mut self,
        req: Request<<Self as hyper::service::Service>::ReqBody>,
    ) -> <Self as hyper::service::Service>::Future {
        // Check URI.
        if req.uri() != "/metrics" {
            return Box::new(future::ok(
                Response::builder()
                    .status(StatusCode::MOVED_PERMANENTLY)
                    .header(LOCATION, "/metrics")
                    .body(Body::empty())
                    .unwrap(),
            ));
        }

        // Check authentication.
        if !check_auth(&req, &self.username, &self.password) {
            return Box::new(future::ok(
                Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(WWW_AUTHENTICATE, "Basic realm=\"Use username metrics and user-defined password to access metrics.\" charset=\"UTF-8\"")
                    .body(Body::empty())
                    .unwrap()
            ));
        }

        Box::new(future::ok(Response::new(self.serve())))
    }
}
