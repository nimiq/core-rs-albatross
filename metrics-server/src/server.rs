use std::{net::SocketAddr, sync::Arc};

use bytes::Bytes;
use futures::future::BoxFuture;
use http_body_util::Full;
use hyper::{
    body::Incoming as IncomingBody, http::StatusCode, service::Service, Method, Request, Response,
};
use hyper_util::{
    rt::tokio::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use log::{error, info};
use nimiq_utils::spawn;
use parking_lot::RwLock;
use prometheus_client::{encoding::text::encode, registry::Registry};
use tokio::net::TcpListener;

pub async fn metrics_server(addr: SocketAddr, registry: Registry) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(&addr).await?;
    info!("Metrics server on http://{}/metrics", addr);
    let metrics_service = MetricService::new(registry);
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let svc_clone = metrics_service.clone();
        spawn(async move {
            if let Err(error) = Builder::new(TokioExecutor::new())
                .serve_connection(io, svc_clone)
                .await
            {
                error!(%error, "server error",);
            }
        });
    }
}

#[derive(Debug, Clone)]
pub struct MetricService {
    reg: Arc<RwLock<Registry>>,
}

type SharedRegistry = Arc<RwLock<Registry>>;

impl MetricService {
    pub fn new(registry: Registry) -> Self {
        Self {
            reg: Arc::new(RwLock::new(registry)),
        }
    }
    fn get_reg(&self) -> SharedRegistry {
        Arc::clone(&self.reg)
    }
    fn respond_with_metrics(&self) -> Response<Full<Bytes>> {
        let mut encoded = String::new();
        let reg = self.get_reg();
        encode(&mut encoded, &reg.read()).unwrap();
        let metrics_content_type = "application/openmetrics-text;charset=utf-8;version=1.0.0";
        Response::builder()
            .status(StatusCode::OK)
            .header(hyper::header::CONTENT_TYPE, metrics_content_type)
            .body(Full::new(Bytes::from(encoded)))
            .unwrap()
    }
    fn respond_with_404_not_found(&self) -> Response<Full<Bytes>> {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from(
                "Not found try localhost:[port]/metrics",
            )))
            .unwrap()
    }
}

impl Service<Request<IncomingBody>> for MetricService {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<IncomingBody>) -> Self::Future {
        let req_path = req.uri().path();
        let req_method = req.method();
        let resp = if (req_method == Method::GET) && (req_path == "/metrics") {
            // Encode and serve metrics from registry.
            self.respond_with_metrics()
        } else {
            self.respond_with_404_not_found()
        };
        Box::pin(async { Ok(resp) })
    }
}
