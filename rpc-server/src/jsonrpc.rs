use std::str::from_utf8;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{FutureExt, stream::Stream, StreamExt};
use futures::future::BoxFuture;
use headers::HeaderMapExt;
use hyper::{Body, Method, Request, Response, StatusCode};
use hyper::header::{HeaderName, HeaderMap, HeaderValue};
use json::{Array, JsonValue, Null, array, object};

use crate::error::AuthenticationError;
use crate::websocket::WsRpcServer;

pub trait Handler: Send + Sync {
    fn call_method(&self, name: &str, params: Array) -> Option<Result<JsonValue, JsonValue>>;
    fn authorize(&self, _username: &str, _password: &str) -> Result<(), AuthenticationError> {
        Ok(())
    }
}

pub struct Service<H> where H: Handler {
    handler: Arc<H>,
    ws_handler: Arc<WsRpcServer>,
}

impl<H> Service<H> where H: Handler {
    pub fn new(handler: Arc<H>, ws_handler: Arc<WsRpcServer>) -> Self {
        Service {
            handler,
            ws_handler,
        }
    }

    // Compute the Sec-WebSocket-Accept header.
    // Returns None if client wishes to keep a HTTP connections.
    // Returns an error if neither HTTP or WS is wanted.
    pub(crate) fn get_ws_upgrade(headers: &HeaderMap) -> Option<Result<headers::SecWebsocketKey, ()>> {
        if !Self::has_header_value(headers, hyper::header::CONNECTION, "Upgrade") {
            return None;
        }
        if !Self::has_header_value(headers, hyper::header::UPGRADE, "WebSocket") {
            return Some(Err(()));
        }
        Some(headers.typed_get().ok_or(()))
    }

    fn has_header_value(headers: &HeaderMap<HeaderValue>, key: HeaderName, expected: &str) -> bool {
        match headers.get(key) {
            None => false,
            Some(actual) => {
                // Only accept ASCII
                let actual_bytes = actual.as_bytes();
                for byte in actual_bytes {
                    if *byte < 0x20 || *byte >= 0x80 {
                        return false;
                    }
                }
                let actual_str = if let Ok(str) = std::str::from_utf8(actual.as_bytes()) {
                    str
                } else {
                    return false;
                };
                actual_str.to_ascii_lowercase() == expected.to_ascii_lowercase()
            },
        }
    }
}

fn handle_request<H>(handler: Arc<H>, str_o: Result<&str, std::str::Utf8Error>) -> Response<Body> where H: Handler {
    let builder = Response::builder()
        .header("Content-Type", "application/json");
    if str_o.is_err() {
        return builder
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(json::stringify(object! {
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
            .body(Body::from(json::stringify(object! {
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
            .body(Body::from(json::stringify(object! {
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
            results.push(object! {
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
            params_array,
        );
        if result_o.is_none() {
            warn!("Unknown method called: {}", msg["method"]);
            results.push(object! {
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
            Ok(result) => object! {
                "jsonrpc" => "2.0",
                "id" => msg["id"].clone(),
                "result" => result
            },
            Err(error) => object! {
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
    } else {
        handler.authorize("", "")
    }
}

impl<H> hyper::service::Service<Request<Body>> for Service<H> where H: Handler + 'static {
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let handler = Arc::clone(&self.handler);

        let headers = req.headers();
        if let Some(ws_key_res) = Self::get_ws_upgrade(headers) {
            let ws_key = match ws_key_res {
                Err(_) => return ok_response_fut(
                    Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::from("Only WS or HTTP supported"))
                        .unwrap()
                ),
                Ok(ws_key) => ws_key,
            };
            let ws_server = Arc::clone(&self.ws_handler);
            tokio::spawn(async move {
                let upgrade = req.into_body().on_upgrade().await;
                let conn = match upgrade {
                    Err(e) => {
                        warn!("Upgrade failed: {}", e);
                        return;
                    },
                    Ok(conn) => conn,
                };
                let ws_stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                    conn,
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                ).await;
                ws_server.handle(ws_stream).await;
            });
            let mut upgrade_response = Response::builder()
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .body(Body::empty())
                .unwrap();
            let headers = upgrade_response.headers_mut();
            headers.typed_insert(headers::Connection::upgrade());
            headers.typed_insert(headers::Upgrade::websocket());
            headers.typed_insert(headers::SecWebsocketAccept::from(ws_key));
            return ok_response_fut(upgrade_response);
        }

        match *req.method() {
            Method::GET => ok_response_fut(Response::new(Body::from("Nimiq JSON-RPC Server"))),
            Method::POST => {
                if let Err(e) = check_authentication(Arc::clone(&handler), req.headers().get("Authorization")) {
                    info!("Authentication failed: {}", e);
                    return async move {
                        Ok(Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Body::from(""))
                            .unwrap())
                    }.boxed();
                }
                async move {
                    let mut stream = req.into_body();
                    let size_hint = stream.size_hint();
                    let size_guess = size_hint.1.unwrap_or(size_hint.0);
                    let mut buf = Vec::<u8>::with_capacity(size_guess);
                    while let Some(chunk_res) = stream.next().await {
                        let chunk = chunk_res?;
                        buf.extend(chunk.to_vec());
                    }
                    Ok(handle_request(handler, from_utf8(&buf)))
                }.boxed()
            },
            _ => ok_response_fut(Response::new(Body::from(""))),
        }
    }
}

fn ok_response_fut(res: Response<Body>) -> BoxFuture<'static, Result<Response<Body>, hyper::Error>> {
    async move { Ok(res) }.boxed()
}
