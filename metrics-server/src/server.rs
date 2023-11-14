use std::{
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
};

use futures::future::BoxFuture;
use hyper::{http::StatusCode, service::Service, Body, Method, Request, Response, Server};
use log::{error, info};
use parking_lot::RwLock;
use prometheus_client::{encoding::text::encode, registry::Registry};

pub async fn metrics_server(addr: SocketAddr, registry: Registry) -> Result<(), std::io::Error> {
    let server = Server::bind(&addr).serve(MakeMetricService::new(registry));
    info!("Metrics server on http://{}/metrics", server.local_addr());
    if let Err(e) = server.await {
        error!("server error: {}", e);
    }
    Ok(())
}

pub struct MetricService {
    reg: Arc<RwLock<Registry>>,
}

type SharedRegistry = Arc<RwLock<Registry>>;

impl MetricService {
    fn get_reg(&mut self) -> SharedRegistry {
        Arc::clone(&self.reg)
    }
    fn respond_with_metrics(&mut self) -> Response<Body> {
        let mut encoded = String::new();
        let reg = self.get_reg();
        encode(&mut encoded, &reg.read()).unwrap();
        let metrics_content_type = "application/openmetrics-text;charset=utf-8;version=1.0.0";
        Response::builder()
            .status(StatusCode::OK)
            .header(hyper::header::CONTENT_TYPE, metrics_content_type)
            .body(Body::from(encoded))
            .unwrap()
    }
    fn respond_with_404_not_found(&mut self) -> Response<Body> {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found try localhost:[port]/metrics"))
            .unwrap()
    }
}

impl Service<Request<Body>> for MetricService {
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
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

pub struct MakeMetricService {
    reg: SharedRegistry,
}

impl MakeMetricService {
    pub fn new(registry: Registry) -> MakeMetricService {
        MakeMetricService {
            reg: Arc::new(RwLock::new(registry)),
        }
    }
}

impl<T> Service<T> for MakeMetricService {
    type Response = MetricService;
    type Error = hyper::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: T) -> Self::Future {
        let reg = self.reg.clone();
        let fut = async move { Ok(MetricService { reg }) };
        Box::pin(fut)
    }
}
