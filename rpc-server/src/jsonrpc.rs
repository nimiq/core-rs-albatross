use futures::{Async, future, Future, IntoFuture, stream::Stream};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use json::{Array, JsonValue, Null};
use std::collections::HashMap;
use std::sync::Arc;

pub trait Handler: Send + Sync {
    fn get_method(&self, name: &str) -> Option<fn(&Self, params: Array) -> Result<JsonValue, JsonValue>>;
}

pub struct Service<H> where H: Handler {
    handler: Arc<H>
}

impl<H> Service<H> where H: Handler {
    pub fn new(handler: H) -> Self {
        Service {
            handler: Arc::new(handler)
        }
    }
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

fn handle_request<H>(handler: Arc<H>, str_o: Result<&str, std::str::Utf8Error>) -> Response<Body> where H: Handler {
    if str_o.is_err() {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(json::stringify(object!{
                            "jsonrpc" => "2.0",
                            "id" => Null,
                            "error" => object!{
                                "code" => -32600,
                                "message" => "Invalid encoding"
                            }
                        })))
            .unwrap();
    }
    let json_o = json::parse(str_o.unwrap());
    if json_o.is_err() {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(json::stringify(object!{
                            "jsonrpc" => "2.0",
                            "id" => Null,
                            "error" => object!{
                                "code" => -32600,
                                "message" => "Invalid JSON"
                            }
                        })))
            .unwrap();
    }
    let mut json = json_o.unwrap();
    let single = json.is_object();
    if single {
        json = array![json];
    }
    if !json.is_array() {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(json::stringify(object!{
                            "jsonrpc" => "2.0",
                            "id" => Null,
                            "error" => object!{
                                "code" => -32600,
                                "message" => "Invalid request"
                            }
                        })))
            .unwrap();
    }
    let mut results = vec![];
    for msg in json.members() {
        if msg["jsonrpc"] != "2.0" || !msg.has_key("method") || !msg["method"].is_string() {
            results.push(object!{
                            "jsonrpc" => "2.0",
                            "id" => msg["id"].clone(),
                            "error" => object!{
                                "code" => -32600,
                                "message" => "Invalid request"
                            }
                        });
            continue;
        }
        let method_o = handler.get_method(msg["method"].as_str().unwrap());
        if method_o.is_none() {
            warn!("Unknown method called: {}", msg["method"]);
            results.push(object!{
                            "jsonrpc" => "2.0",
                            "id" => msg["id"].clone(),
                            "error" => object!{
                                "code" => -32601,
                                "message" => "Method not found"
                            }
                        });
            continue;
        }
        let method = method_o.unwrap();
        let params = msg["params"].clone();
        let params_array = match params {
            JsonValue::Array(a) => a,
            _ => vec![params]
        };

        results.push(match method(&handler, params_array) {
            Ok(result) => object!{
                            "jsonrpc" => "2.0",
                            "id" => msg["id"].clone(),
                            "result" => result
                        },
            Err(error) => object!{
                            "jsonrpc" => "2.0",
                            "id" => msg["id"].clone(),
                            "error" => error
                        }
        });
    }

    if single {
        Response::new(Body::from(results.pop().map(json::stringify).unwrap_or("".to_string())))
    } else {
        Response::new(Body::from(json::stringify(JsonValue::Array(results))))
    }
}

impl<H> IntoFuture for Service<H> where H: Handler {
    type Future = future::FutureResult<Self::Item, Self::Error>;
    type Item = Self;
    type Error = Never;

    fn into_future(self) -> Self::Future {
        future::ok(self)
    }
}

impl<H> hyper::service::Service for Service<H> where H: Handler + 'static {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = Box<Future<Item=Response<Body>, Error=hyper::Error> + Send>;

    fn call(&mut self, req: Request<<Self as hyper::service::Service>::ReqBody>) -> <Self as hyper::service::Service>::Future {
        let handler = Arc::clone(&self.handler);
        match req.method() {
            &Method::GET => Box::new(future::ok(Response::new(Body::from("Nimiq JSON-RPC Server")))),
            &Method::POST => {
                Box::new(req.into_body().concat2()
                    .map(|b| handle_request(handler, std::str::from_utf8(&b))))
            },
            _ => Box::new(future::ok(Response::new(Body::from(""))))
        }
    }
}
