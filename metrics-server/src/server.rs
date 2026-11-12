use std::{
    fmt::{self, Debug},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use futures::future::BoxFuture;
use headers::{authorization::Basic, Authorization, HeaderMapExt};
use http_body_util::Full;
use hyper::{
    body::Incoming as IncomingBody, header, http::StatusCode, service::Service, Method, Request,
    Response,
};
use hyper_util::{
    rt::tokio::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto::Builder,
};
use log::{debug, error};
use nimiq_time::sleep;
use nimiq_utils::{spawn, Credentials};
use parking_lot::RwLock;
use prometheus_client::{encoding::text::encode, registry::Registry};
use tokio::net::TcpListener;

/// How long to wait before accepting again after `accept()` failed, so that a persistent
/// error (such as running out of file descriptors) does not turn into a busy loop.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// How long a client may take to send the request headers, measured from when the connection
/// starts waiting for a request (including idle keep-alive connections). Without it, a client
/// that connects and never sends anything holds its connection and task forever.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Reports whether the node is ready to serve, for the `/ready` endpoint.
pub type ReadinessCheck = Arc<dyn Fn() -> bool + Send + Sync>;

pub async fn metrics_server(
    listener: TcpListener,
    registry: Registry,
    credentials: Option<Credentials>,
    is_ready: ReadinessCheck,
) {
    serve(
        listener,
        MetricService::new(registry, credentials, is_ready),
        HEADER_READ_TIMEOUT,
    )
    .await
}

async fn serve(
    listener: TcpListener,
    metrics_service: MetricService,
    header_read_timeout: Duration,
) {
    // Scrapers talk HTTP/1.1 to plain-HTTP targets. Serving HTTP/1 only also skips the
    // protocol detection of the auto builder, which would wait for the first bytes without
    // any timeout before the header read timeout applies.
    let mut builder = Builder::new(TokioExecutor::new()).http1_only();
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout);
    let builder = Arc::new(builder);

    loop {
        // An accept error (e.g. the process ran out of file descriptors) must not take
        // the metrics server down for the rest of the node's lifetime.
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                error!(%error, "Metrics server failed to accept a connection");
                sleep(ACCEPT_ERROR_BACKOFF).await;
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let svc_clone = metrics_service.clone();
        let builder = Arc::clone(&builder);
        spawn(async move {
            if let Err(error) = builder.serve_connection(io, svc_clone).await {
                // Idle keep-alive connections between scrapes end this way, so it is routine.
                if error
                    .downcast_ref::<hyper::Error>()
                    .is_some_and(hyper::Error::is_timeout)
                {
                    debug!(%error, "Metrics server closed a connection that sent no request");
                } else {
                    error!(%error, "server error",);
                }
            }
        });
    }
}

#[derive(Clone)]
pub struct MetricService {
    reg: Arc<RwLock<Registry>>,
    credentials: Option<Arc<Credentials>>,
    is_ready: ReadinessCheck,
}

impl Debug for MetricService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricService")
            .field("reg", &self.reg)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

type SharedRegistry = Arc<RwLock<Registry>>;

impl MetricService {
    pub fn new(
        registry: Registry,
        credentials: Option<Credentials>,
        is_ready: ReadinessCheck,
    ) -> Self {
        Self {
            reg: Arc::new(RwLock::new(registry)),
            credentials: credentials.map(Arc::new),
            is_ready,
        }
    }
    fn get_reg(&self) -> SharedRegistry {
        Arc::clone(&self.reg)
    }
    fn respond_with_metrics(&self) -> Response<Full<Bytes>> {
        let mut encoded = String::new();
        let reg = self.get_reg();
        if let Err(error) = encode(&mut encoded, &reg.read()) {
            // A metric failed to encode (e.g. a collector returned an error). Report it to the
            // scraper, so the target shows up as failing, instead of panicking the connection.
            error!(%error, "Failed to encode metrics");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from("Failed to encode metrics")))
                .unwrap();
        }
        let metrics_content_type = "application/openmetrics-text;charset=utf-8;version=1.0.0";
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, metrics_content_type)
            .body(Full::new(Bytes::from(encoded)))
            .unwrap()
    }
    fn respond_with_text(status: StatusCode, text: &'static str) -> Response<Full<Bytes>> {
        Response::builder()
            .status(status)
            .body(Full::new(Bytes::from(text)))
            .unwrap()
    }
    fn respond_with_readiness(&self) -> Response<Full<Bytes>> {
        if (self.is_ready)() {
            Self::respond_with_text(StatusCode::OK, "Ready")
        } else {
            Self::respond_with_text(StatusCode::SERVICE_UNAVAILABLE, "Consensus not established")
        }
    }
    fn respond_with_404_not_found(&self) -> Response<Full<Bytes>> {
        Self::respond_with_text(
            StatusCode::NOT_FOUND,
            "Not found. Available paths: /metrics, /health, /ready",
        )
    }
    fn respond_with_401_unauthorized(&self) -> Response<Full<Bytes>> {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::WWW_AUTHENTICATE, "Basic realm=\"metrics\"")
            .body(Full::new(Bytes::from("Unauthorized")))
            .unwrap()
    }

    fn is_authorized(&self, req: &Request<IncomingBody>) -> bool {
        let Some(expected) = self.credentials.as_ref() else {
            return true;
        };
        let Some(Authorization(basic)) = req.headers().typed_get::<Authorization<Basic>>() else {
            return false;
        };
        expected.check(basic.username(), basic.password())
    }
}

impl Service<Request<IncomingBody>> for MetricService {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<IncomingBody>) -> Self::Future {
        let resp = match (req.method(), req.uri().path()) {
            (&Method::GET, "/metrics") => {
                if !self.is_authorized(&req) {
                    self.respond_with_401_unauthorized()
                } else {
                    self.respond_with_metrics()
                }
            }
            // Liveness and readiness probes (e.g. Docker, Kubernetes) are unauthenticated:
            // they reveal nothing but the node's state and probes rarely support credentials.
            (&Method::GET, "/health") => Self::respond_with_text(StatusCode::OK, "OK"),
            (&Method::GET, "/ready") => self.respond_with_readiness(),
            _ => self.respond_with_404_not_found(),
        };
        Box::pin(async { Ok(resp) })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::SocketAddr,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };

    use hyper::http::StatusCode;
    use nimiq_utils::{spawn, spawn_blocking, Credentials};
    use prometheus_client::{
        collector::Collector, encoding::DescriptorEncoder, registry::Registry,
    };
    use tokio::net::TcpListener;

    use super::{serve, MetricService, ReadinessCheck, HEADER_READ_TIMEOUT};

    async fn start(credentials: Option<Credentials>, is_ready: ReadinessCheck) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let service = MetricService::new(Registry::default(), credentials, is_ready);
        spawn(serve(listener, service, HEADER_READ_TIMEOUT));
        addr
    }

    /// Sends `GET path` and returns the response's status line.
    async fn get_status(addr: SocketAddr, path: &'static str) -> String {
        spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response.lines().next().unwrap_or_default().to_owned()
        })
        .await
    }

    #[tokio::test]
    async fn serves_health_and_readiness_without_credentials() {
        let ready = Arc::new(AtomicBool::new(false));
        let is_ready = {
            let ready = Arc::clone(&ready);
            Arc::new(move || ready.load(Ordering::Relaxed))
        };
        let credentials = Credentials::new("user", "secret");
        let addr = start(Some(credentials), is_ready).await;

        assert_eq!(get_status(addr, "/health").await, "HTTP/1.1 200 OK");
        assert_eq!(
            get_status(addr, "/ready").await,
            "HTTP/1.1 503 Service Unavailable"
        );
        ready.store(true, Ordering::Relaxed);
        assert_eq!(get_status(addr, "/ready").await, "HTTP/1.1 200 OK");

        assert_eq!(
            get_status(addr, "/metrics").await,
            "HTTP/1.1 401 Unauthorized"
        );
        assert_eq!(get_status(addr, "/other").await, "HTTP/1.1 404 Not Found");
    }

    #[tokio::test]
    async fn closes_connections_that_send_no_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let service = MetricService::new(Registry::default(), None, Arc::new(|| true));
        spawn(serve(listener, service, Duration::from_millis(100)));

        let read = spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.read(&mut [0u8; 1])
        })
        .await;

        assert_eq!(read.unwrap(), 0, "the server should close the connection");
    }

    #[derive(Debug)]
    struct FailingCollector;

    impl Collector for FailingCollector {
        fn encode(&self, _encoder: DescriptorEncoder) -> Result<(), std::fmt::Error> {
            Err(std::fmt::Error)
        }
    }

    #[test]
    fn answers_500_when_encoding_fails() {
        let mut registry = Registry::default();
        registry.register_collector(Box::new(FailingCollector));
        let service = MetricService::new(registry, None, Arc::new(|| true));

        let response = service.respond_with_metrics();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
