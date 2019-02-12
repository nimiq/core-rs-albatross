#[macro_use]
extern crate json;
#[macro_use]
extern crate log;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_accounts as accounts;
extern crate nimiq_consensus as consensus;
extern crate nimiq_network as network;
extern crate nimiq_hash as hash;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;

use std::str::FromStr;
use std::sync::Arc;
use std::net::{SocketAddr, IpAddr};

use futures::future::Future;
use hyper::Server;
use json::{Array, JsonValue, Null, };

use beserial::Serialize;
use consensus::consensus::{Consensus, ConsensusEvent};
use hash::{Argon2dHash, Blake2bHash, Hash};
use primitives::block::{Block, Difficulty};
use primitives::transaction::Transaction;
use crate::error::Error;

pub mod jsonrpc;
pub mod error;

pub struct JsonRpcHandler {
    consensus: Arc<Consensus>,
    consensus_state: &'static str,
    starting_block: u32,
}

impl JsonRpcHandler {
    pub fn new(consensus: Arc<Consensus>) -> Self {
        let mut res = JsonRpcHandler {
            consensus: consensus.clone(),
            consensus_state: "syncing",
            starting_block: consensus.blockchain.height(),
        };

        // Listen for consensus events
        /*
        TODO: We might need an Arc to the JsonRpcHandler here
        consensus.notifier.write().register(|e: ConsensusEvent| {
            match e {
                Established => { res.consensus_state = "established" },
                Lost => { res.consensus_state = "lost" },
                Syncing => { res.consensus_state = "syncing" },
                _ => ()
            }
        });
        */

        res
    }


    fn peer_count(&self, _: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.network.peer_count().into())
    }

    fn consensus(&self, _: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus_state.into())
    }

    fn syncing(&self, _: Array) -> Result<JsonValue, JsonValue> {
        Ok(if self.consensus_state == "established" {
            false.into()
        }
        else {
            let current_block = self.consensus.blockchain.height();
            object! {
                "starting_block" => self.starting_block,
                "current_block" => current_block,
                "highest_block" => current_block // TODO
            }
        })
    }

    fn peer_list(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok("TODO".into())
    }

    fn peer_state(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok("TODO".into())
    }

    fn block_number(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.blockchain.height().into())
    }

    fn get_block_transaction_count_by_hash(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_by_hash(params.get(0).unwrap_or(&Null))?
            .body.ok_or_else(|| object!{"message" => "No body for block found"})?
            .transactions.len().into())
    }

    fn get_block_transaction_count_by_number(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_by_number(params.get(0).unwrap_or(&Null))?
            .body.ok_or_else(|| object!{"message" => "No body for block found"})?
            .transactions.len().into())
    }

    fn get_block_by_hash(&self, params: Array) -> Result<JsonValue, JsonValue> {
        self.block_to_obj(&self.block_by_hash(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false))
    }

    fn get_block_by_number(&self, params: Array) -> Result<JsonValue, JsonValue> {
        self.block_to_obj(&self.block_by_number(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false))
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
        self.consensus.blockchain
            .get_block_at(block_number, true)
            .ok_or_else(|| object!{"message" => "Block not found"})
    }

    fn block_by_hash(&self, hash: &JsonValue) -> Result<Block, JsonValue> {
        let hash = hash.as_str()
            .ok_or_else(|| object!{"message" => "Hash must be a string"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid Blake2b hash"}))?;
        self.consensus.blockchain.get_block(&hash, false, true)
            .ok_or_else(|| object!{"message" => "Block not found"})
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
            "minerAddress" => block.body.as_ref().map(|body| body.miner.to_user_friendly_address().into()).unwrap_or(Null),
            "difficulty" => Difficulty::from(block.header.n_bits).to_string(),
            "extraData" => block.body.as_ref().map(|body| hex::encode(&body.extra_data).into()).unwrap_or(Null),
            "size" => block.serialized_size(),
            "timestamp" => block.header.timestamp,
            "transactions" => JsonValue::Array(block.body.as_ref().map(|body| if include_transactions { 
                body.transactions.iter().enumerate().map(|(i, tx)| self.transaction_to_obj(tx, Some(block), i).unwrap_or(Null)).collect()
            } else { 
                body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
            }).unwrap_or(vec![])),
        })
    }
    
    fn transaction_to_obj(&self, transaction: &Transaction, block: Option<&Block>, i: usize) -> Result<JsonValue, JsonValue> {
        let header = block.as_ref().map(|b| &b.header);
        Ok(object!{
            "hash" => transaction.hash::<Blake2bHash>().to_hex(),
            "blockHash" => header.map(|h| h.hash::<Blake2bHash>().to_hex().into()).unwrap_or(Null),
            "blockNumber" => header.map(|h| h.height.into()).unwrap_or(Null),
            "blockHash" => header.map(|h| h.timestamp.into()).unwrap_or(Null),
            "confirmations" => header.map(|b| (self.consensus.blockchain.height() - b.height).into()).unwrap_or(Null),
            "transactionIndex" => i,
            "from" => transaction.sender.to_hex(),
            "fromAddress" => transaction.sender.to_user_friendly_address(),
            "to" => transaction.recipient.to_hex(),
            "toAddress" => transaction.recipient.to_user_friendly_address(),
            "value" => u64::from(transaction.value),
            "fee" => u64::from(transaction.fee),
            "data" => hex::encode(&transaction.data),
            "flags" => transaction.flags.bits()
        })
    }
}

impl jsonrpc::Handler for JsonRpcHandler {
    fn get_method(&self, name: &str) -> Option<fn(&Self, Array) -> Result<JsonValue, JsonValue>> {
        // TODO: Apply method white-listing
        match name {
            // Network
            "peerCount" => Some(JsonRpcHandler::peer_count),
            "syncing" => Some(JsonRpcHandler::syncing),
            "consensus" => Some(JsonRpcHandler::consensus),
            "peerCount" => Some(JsonRpcHandler::peer_list),
            "peerState" => Some(JsonRpcHandler::peer_count),

            // Transactions
            // TODO

            // Blockchain
            "blockNumber" => Some(JsonRpcHandler::block_number),
            "getBlockTransactionCountByHash" => Some(JsonRpcHandler::get_block_transaction_count_by_hash),
            "getBlockTransactionCountByNumber" => Some(JsonRpcHandler::get_block_transaction_count_by_number),
            "getBlockByHash" => Some(JsonRpcHandler::get_block_by_hash),
            "getBlockByNumber" => Some(JsonRpcHandler::get_block_by_number),

            _ => None
        }
    }
}


pub fn rpc_server(consensus: Arc<Consensus>, ip: IpAddr, port: u16) -> Result<Box<dyn Future<Item=(), Error=()> + Send + Sync>, Error> {
    Ok(Box::new(Server::try_bind(&SocketAddr::new(ip, port))?
        .serve(move || {
            jsonrpc::Service::new(JsonRpcHandler::new(Arc::clone(&consensus)))
        })
        .map_err(|e| error!("RPC server failed: {}", e)))) // as Box<dyn Future<Item=(), Error=()> + Send + Sync>
}
