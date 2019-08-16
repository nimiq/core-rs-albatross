use std::sync::Arc;

use futures::{future, Future, IntoFuture, stream::Stream};
use hyper::{Body, Method, Request, Response, StatusCode};
use hyper::header::HeaderValue;
use json::{Array, JsonValue, Null};

use crate::error::AuthenticationError;

pub trait Handler: Send + Sync {
    fn call_method(&self, name: &str, params: Array) -> Option<Result<JsonValue, JsonValue>>;
    fn authorize(&self, _username: &str, _password: &str) -> Result<(), AuthenticationError> {
        Ok(())
    }
}

pub struct Service<H> where H: Handler {
    handler: Arc<H>
}

impl<H> Service<H> where H: Handler {
    pub fn new(handler: Arc<H>) -> Self {
        Service {
            handler,
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
    let mut builder = Response::builder();
    builder.header("Content-Type", "application/json");
    if str_o.is_err() {
        return builder
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
        return builder
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
        return builder
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

        let params = msg["params"].clone();
        let params_array = match params {
            JsonValue::Array(a) => a,
            _ => vec![params]
        };

        let result_o = handler.call_method(
            msg["method"].as_str().unwrap(),
            params_array
        );
        if result_o.is_none() {
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

        results.push(match result_o.unwrap() {
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
        builder.body(Body::from(results.pop().map(json::stringify).unwrap_or_else(String::new))).unwrap()
    } else {
        builder.body(Body::from(json::stringify(JsonValue::Array(results)))).unwrap()
    }
}

fn check_authentication<H: Handler>(handler: Arc<H>, authorization: Option<&HeaderValue>) -> Result<(), AuthenticationError> {
    if let Some(authorization) = authorization {
        let authorization = authorization.to_str()
            .map_err(|_| AuthenticationError::InvalidHeader)?
            .split_whitespace().collect::<Vec<&str>>();
        if authorization.len() != 2 || authorization[0] != "Basic" {
            return Err(AuthenticationError::InvalidHeader);
        }
        let authorization = base64::decode(authorization[1])
            .map_err(|_| AuthenticationError::InvalidHeader)?;
        let authorization = std::str::from_utf8(authorization
            .as_slice()).map_err(|_| AuthenticationError::InvalidHeader)?
            .split(':').collect::<Vec<&str>>();
        if authorization.len() != 2 {
            return Err(AuthenticationError::IncorrectCredentials);
        }
        handler.authorize(authorization[0], authorization[1])
    }
    else {
        handler.authorize("", "")
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
    type Future = Box<dyn Future<Item=Response<Body>, Error=hyper::Error> + Send>;

    fn call(&mut self, req: Request<<Self as hyper::service::Service>::ReqBody>) -> <Self as hyper::service::Service>::Future {
        let handler = Arc::clone(&self.handler);
        match *req.method() {
            Method::GET => Box::new(future::ok(Response::new(Body::from("Nimiq JSON-RPC Server")))),
            Method::POST => {
                if let Err(e) = check_authentication(Arc::clone(&handler), req.headers().get("Authorization")) {
                    info!("Authentication failed: {}", e);
                    //return Box::new(future::ok(Response::new(Body::from(json::stringify(e)))));
                    return Box::new(future::ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::from(""))
                        .unwrap()))
                }
                Box::new(req.into_body().concat2()
                    .map(|b| handle_request(handler, std::str::from_utf8(&b))))
            },
            _ => Box::new(future::ok(Response::new(Body::from(""))))
        }
    }
}
