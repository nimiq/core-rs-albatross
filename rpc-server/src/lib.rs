#[macro_use]
extern crate json;
//#[macro_use]
//extern crate lazy_static;
#[macro_use]
extern crate log;
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

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::future::Future;
use hyper::Server;
use json::JsonValue;

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
    pub credentials: Option<Credentials>,
    pub methods: HashSet<String>,
    pub allowip: (),
    pub corsdomain: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    pub fn new(username: &str, password: &str) -> Credentials {
        Credentials { username: String::from(username), password: String::from(password) }
    }

    pub fn check(&self, username: &str, password: &str) -> bool {
        self.username == username && self.password == password
    }
}

type OtherFuture = Box<dyn Future<Item=(), Error=()> + Send + Sync + 'static>;

pub fn rpc_server(ip: IpAddr, port: u16, handler: Arc<Handler>) -> Result<OtherFuture, Error> {
    Ok(Box::new(Server::try_bind(&SocketAddr::new(ip, port))?
        .serve(move || {
            jsonrpc::Service::new(Arc::clone(&handler))
        })
        .map_err(|e| error!("RPC server failed: {}", e)))) // as Box<dyn Future<Item=(), Error=()> + Send + Sync>
}
