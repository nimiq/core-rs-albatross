#[macro_use]
extern crate json;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_accounts as accounts;
extern crate nimiq_consensus as consensus;
extern crate nimiq_network as network;
extern crate nimiq_database as database;
extern crate nimiq_hash as hash;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;

use std::str::FromStr;
use std::sync::Arc;
use std::error::Error;
use std::net::{SocketAddr, IpAddr};

use futures::{Async, future::Future, stream::Stream};
use hyper::Server;
use json::{Array, JsonValue, Null};
use lmdb_zero::open::Flags;

use beserial::Serialize;
use consensus::consensus::Consensus;
use database::Environment;
use database::lmdb::LmdbEnvironment;
use hash::{Argon2dHash, Blake2bHash, Hash};
use network::network::Network;
use network::network_config::NetworkConfig;
use network_primitives::networks::get_network_info;
use primitives::block::{Block, Difficulty};
use primitives::networks::NetworkId;
use primitives::transaction::Transaction;

pub mod jsonrpc;


pub struct JsonRpcHandler {
    consensus: Arc<Consensus>,
    consensus_state: &'static str
}

impl JsonRpcHandler {
    pub fn new(consensus: Arc<Consensus>) -> Self {
        let res = JsonRpcHandler { 
            consensus,
            consensus_state: "syncing"
        };
        // TODO: Listen for consensus events
        res
    }
    
    fn get_block_by_number(&self, params: Array) -> Result<JsonValue, JsonValue> {
        self.block_to_obj(&self.block_by_number(params.get(0).unwrap_or(&Null))?, true)
    }
    
    fn block_by_number(&self, number: &JsonValue) -> Result<Block, JsonValue> {
        let mut block_number = if number.is_string() {
            if number.as_str().unwrap().starts_with("latest-") {
                self.consensus.blockchain.height() - u32::from_str(&number.as_str().unwrap()[7..]).map_err(|_| object!{"message" => "Invalid block number"})?
            } else if number.as_str().unwrap() == "latest" {
                self.consensus.blockchain.height()
            } else {
                u32::from_str(number.as_str().unwrap()).map_err(|_| object!{"message" => "Invalid block number"})?
            }
        } else if number.is_number() {
            number.as_u32().ok_or_else(|| object!{"message" => "Invalid block number"})?
        } else {
            return Err(object!{"message" => "Invalid block number"});
        };
        if block_number == 0 {
            block_number = 1;
        }
        self.consensus.blockchain.block_at(block_number, true).ok_or_else(|| object!{"message" => "Block not found"})
    }
    
    fn block_to_obj(&self, block: &Block, include_transactions: bool) -> Result<JsonValue, JsonValue> {
        Ok(object!{
            "number" => block.header.height,
            "hash" => block.header.hash::<Blake2bHash>().to_hex(),
            "pow" => block.header.hash::<Argon2dHash>().to_hex(),
            "parentHash" => block.header.prev_hash.to_hex(),
            "nonce" => block.header.nonce,
            "bodyHash" => block.header.body_hash.to_hex(),
            "accountsHash" => block.header.accounts_hash.to_hex(),
            "miner" => block.body.as_ref().map(|body| body.miner.to_hex().into()).unwrap_or(Null),
            //"minerAddress" // TODO
            "difficulty" => Difficulty::from(block.header.n_bits).to_string(),
            "extraData" => block.body.as_ref().map(|body| hex::encode(&body.extra_data).into()).unwrap_or(Null),
            "size" => block.serialized_size(),
            "timestamp" => block.header.timestamp,
            "transactions" => JsonValue::Array(block.body.as_ref().map(|body| if include_transactions { 
                body.transactions.iter().map(|tx| self.transaction_to_obj(tx, block, 0).unwrap_or(Null)).collect()
            } else { 
                body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
            }).unwrap_or(vec![])),
        })
    }
    
    fn transaction_to_obj(&self, transaction: &Transaction, block: &Block, i: usize) -> Result<JsonValue, JsonValue> {
        Ok(Null)
    }

    fn block_number(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.blockchain.height().into())
    }

    fn peer_count(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.network.peer_count().into())
    }

    fn consensus(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus_state.into())
    }
}

impl jsonrpc::Handler for JsonRpcHandler {
    fn get_method(&self, name: &str) -> Option<fn(&Self, Array) -> Result<JsonValue, JsonValue>> {
        match name {
            "getBlockByNumber" => Some(JsonRpcHandler::get_block_by_number),
            "blockNumber" => Some(JsonRpcHandler::block_number),
            "peerCount" => Some(JsonRpcHandler::peer_count),
            "consensus" => Some(JsonRpcHandler::consensus),
            _ => None
        }
    }
}


pub fn rpc_server(consensus: Arc<Consensus>, ip: IpAddr, port: u16) -> Box<dyn Future<Item=(), Error=()> + Send + Sync> {
    Box::new(Server::bind(&SocketAddr::new(ip, port))
        .serve(move || {
            jsonrpc::Service::new(JsonRpcHandler::new(Arc::clone(&consensus)))
        })
        .map_err(|e| error!("RPC server failed: {}", e))) // as Box<dyn Future<Item=(), Error=()> + Send + Sync>
}
