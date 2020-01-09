#[macro_use]
extern crate log;
extern crate nimiq_account as account;
extern crate nimiq_block as block;
extern crate nimiq_block_albatross as block_albatross;
extern crate nimiq_block_base as block_base;
extern crate nimiq_block_production as block_production;
extern crate nimiq_blockchain_albatross as blockchain_albatross;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_bls as bls;
extern crate nimiq_consensus as consensus;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;
#[cfg(feature="validator")]
extern crate nimiq_validator as validator;

use std::collections::HashSet;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::{future::BoxFuture, FutureExt, TryFutureExt};
use hyper::Server as HyperServer;
use hyper::service::make_service_fn;
use json::{JsonValue, object};

use crate::error::Error;
pub use crate::handler::Handler;

pub mod jsonrpc;
pub mod error;
pub mod handler;
pub mod handlers;

fn rpc_not_implemented<T>() -> Result<T, JsonValue> {
    Err(object!{"message" => "Not implemented"})
}


#[derive(Debug, Clone)]
pub struct JsonRpcConfig {
    pub username: Option<String>,
    pub password: Option<String>,
    pub methods: HashSet<String>,
    pub allowip: (),
    pub corsdomain: Vec<String>,
}

pub struct Server {
    pub future: BoxFuture<'static, Result<(), Error>>,
    pub handler: Arc<Handler>,
}

impl Server {
    pub fn new(ip: IpAddr, port: u16, config: JsonRpcConfig) -> Result<Self, Error> {
        let handler = Arc::new(Handler::new(config));
        let handler2 = Arc::clone(&handler);
        let future = HyperServer::try_bind(&SocketAddr::new(ip, port))?
            .serve(make_service_fn(move |_conn| {
                let handler = Arc::clone(&handler2);
                async move {
                    Ok::<_, Infallible>(jsonrpc::Service::new(handler))
                }
            }))
            .map_err(|err| err.into())
            .boxed();
        Ok(Self {
            future,
            handler: Arc::clone(&handler),
        })
    }
}
