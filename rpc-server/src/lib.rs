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
#[cfg(feature = "validator")]
extern crate nimiq_validator as validator;

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::future::Future;
use hyper::Server;
use json::{object, JsonValue};

use crate::error::Error;
pub use crate::handler::Handler;
use futures::IntoFuture;

pub mod error;
pub mod handler;
pub mod handlers;
pub mod jsonrpc;

fn rpc_not_implemented<T>() -> Result<T, JsonValue> {
    Err(object! {"message" => "Not implemented"})
}

#[derive(Debug, Clone)]
pub struct JsonRpcConfig {
    pub username: Option<String>,
    pub password: Option<String>,
    pub methods: HashSet<String>,
    pub allowip: (),
    pub corsdomain: Vec<String>,
}

pub type RpcServerFuture = Box<dyn Future<Item = (), Error = ()> + Send + Sync + 'static>;

pub struct RpcServer {
    future: RpcServerFuture,
    pub handler: Arc<Handler>,
}

impl RpcServer {
    pub fn new(ip: IpAddr, port: u16, config: JsonRpcConfig) -> Result<Self, Error> {
        let handler = Arc::new(Handler::new(config));

        let handler2 = Arc::clone(&handler);
        let future = Box::new(
            Server::try_bind(&SocketAddr::new(ip, port))?
                .serve(move || jsonrpc::Service::new(Arc::clone(&handler2)))
                .map_err(|e| error!("RPC server failed: {}", e)),
        );

        Ok(RpcServer { future, handler })
    }
}

impl IntoFuture for RpcServer {
    type Future = RpcServerFuture;
    type Item = ();
    type Error = ();

    fn into_future(self) -> Self::Future {
        self.future
    }
}
