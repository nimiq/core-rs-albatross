#[macro_use]
extern crate json;
#[macro_use]
extern crate log;
extern crate nimiq_block as block;
extern crate nimiq_consensus as consensus;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_network as network;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_transaction as transaction;

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::collections::{HashSet, HashMap};

use futures::future::Future;
use hyper::Server;
use json::{Array, JsonValue, Null};
use json::object::Object;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use block::{Block, Difficulty};
use consensus::consensus::{Consensus, ConsensusEvent};
use hash::{Argon2dHash, Blake2bHash, Hash};
use hex;
use keys::Address;
use network::address::peer_address_state::{PeerAddressInfo, PeerAddressState};
use network::connection::close_type::CloseType;
use network::connection::connection_info::ConnectionInfo;
use network_primitives::address::{PeerId, PeerUri};
use transaction::{Transaction, TransactionReceipt};
use network::connection::connection_pool::ConnectionId;
use network::peer_scorer::Score;

use crate::error::{Error, AuthenticationError};

pub mod jsonrpc;
pub mod error;


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

pub(crate) struct JsonRpcServerState {
    consensus_state: &'static str,
}

pub(crate) struct JsonRpcHandler {
    state: Arc<RwLock<JsonRpcServerState>>,
    consensus: Arc<Consensus>,
    starting_block: u32,
    config: Arc<JsonRpcConfig>
}

impl JsonRpcHandler {
    pub(crate) fn new(consensus: Arc<Consensus>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self {
        JsonRpcHandler {
            state,
            consensus: consensus.clone(),
            starting_block: consensus.blockchain.height(),
            config
        }
    }


    // Network

    fn peer_count(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.consensus.network.peer_count().into())
    }

    fn consensus(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.state.read().consensus_state.into())
    }

    fn syncing(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(if self.state.read().consensus_state == "established" {
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
        let mut scores: HashMap<ConnectionId, Score> = HashMap::new();
        trace!("[SCORE] Num scores: {}", self.consensus.network.scorer().connection_scores().len());
        for (id, score) in self.consensus.network.scorer().connection_scores() {
            trace!("[SCORE] id={}, score={}", id, score);
            scores.insert(*id, *score);
        }

        Ok(self.consensus.network.addresses.state().address_info_iter()
            .map(|info| {
                let conn_id = self.consensus.network.connections.state()
                    .get_connection_id_by_peer_address(&info.peer_address);
                self.peer_address_info_to_obj(info, None,
                                              conn_id.and_then(|id| scores.get(&id)).map(|s| *s))
            })
            .collect::<Array>().into())
    }

    fn peer_state(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let peer_uri = params.get(0).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Invalid peer URI"})
            .and_then(|uri| PeerUri::from_str(uri)
                .map_err(|e| object!{"message" => e.to_string()}))?;

        let peer_id = peer_uri.peer_id()
            .ok_or_else(|| object!{"message" => "URI must contain peer ID"})
            .and_then(|s| PeerId::from_str(s)
                .map_err(|e| object!{"message" => e.to_string()}))?;

        let mut address_book = self.consensus.network.addresses.state_mut();
        let peer_address = address_book.get_by_peer_id(&peer_id)
            .ok_or_else(|| object!{"message" => "Unknown peer"})?;
        let mut peer_address_info = address_book.get_info_mut(&peer_address)
            .ok_or_else(|| object!{"message" => "Unknown peer"})?;


        let connection_pool = self.consensus.network.connections.state();
        let connection_info = connection_pool.get_connection_by_peer_address(&peer_address_info.peer_address);
        let peer_channel = connection_info.and_then(|c| c.peer_channel());

        let set = params.get(1).unwrap_or(&Null);
        if !set.is_null() {
            let set = set.as_str().ok_or_else(|| object!{"message" => "Invalid value for 'set'"})?;
            match set {
                "disconnect" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerDisconnect));
                },
                "fail" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerFail));
                },
                "ban" => {
                    peer_channel.map(|p| p.close(CloseType::ManualPeerBan));
                },
                "unban" => {
                    if peer_address_info.state == PeerAddressState::Banned {
                        peer_address_info.state = PeerAddressState::Tried;
                    }
                },
                "connect" => {
                    drop(address_book);
                    drop(connection_pool);
                    self.consensus.network.connections.connect_outbound(peer_address);
                }
                _ => return Err(object!{"message" => "Unknown 'set' command."})
            }
            Ok(Null)
        }
        else {
            Ok(self.peer_address_info_to_obj(peer_address_info, connection_info, None))
        }
    }


    // Transaction

    fn send_raw_transaction(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let raw = hex::decode(params.get(0)
                .unwrap_or(&Null)
                .as_str()
                .ok_or_else(|| object!{"message" => "Raw transaction must be a string"} )?)
            .map_err(|_| object!{"message" => "Raw transaction must be a hex string"} )?;
        let transaction: Transaction = Deserialize::deserialize_from_vec(&raw)
            .map_err(|_| object!{"message" => "Transaction can't be deserialized"} )?;
        self.push_transaction(&transaction)
    }

    fn create_raw_transaction(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let raw = Serialize::serialize_to_vec(&self.obj_to_transaction(params.get(0).unwrap_or(&Null))?);
        Ok(hex::encode(raw).into())
    }

    fn send_transaction(&self, params: Array) -> Result<JsonValue, JsonValue> {
        self.push_transaction(&self.obj_to_transaction(params.get(0).unwrap_or(&Null))?)
    }


    fn get_raw_transaction_info(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let transaction: Transaction = params.get(0).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Raw transaction data must be a string"}) // Result<&str, Err>
            .and_then(|s| hex::decode(s)
                .map_err(|_| object!{"message" => "Raw transaction data must be hex-encoded"})) // Result<Vec<u8>, Err>
            .and_then(|b| Deserialize::deserialize_from_vec(&b)
                .map_err(|_| object!{"message" => "Invalid transaction data"}))?;

        let (transaction, valid, in_mempool) =
            if let Ok(live_transaction) = self.get_transaction_by_hash_helper(&transaction.hash::<Blake2bHash>()) {
                let confirmations = live_transaction["confirmations"].as_u32()
                    .expect("Function didn't return transaction with confirmation number");
                (live_transaction, true, confirmations == 0)
            }
            else {
                (self.transaction_to_obj(&transaction, None, None),
                 transaction.verify(self.consensus.blockchain.network_id).is_ok(), false)
            };

        // Insert `valid` and `in_mempool` into `transaction` object.
        match transaction {
            // This should always be an object
            JsonValue::Object(mut o) => {
                o.insert("valid", JsonValue::Boolean(valid));
                o.insert("inMempool", JsonValue::Boolean(in_mempool));
            }
            _ => unreachable!()
        };

        rpc_not_implemented()
    }

    fn get_transaction_by_block_hash_and_index(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_hash(params.get(0).unwrap_or(&Null))?;
        let index = params.get(1).and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        self.get_transaction_by_block_and_index(&block, index)
    }

    fn get_transaction_by_block_number_and_index(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_number(params.get(0).unwrap_or(&Null))?;
        let index = params.get(1).and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        self.get_transaction_by_block_and_index(&block, index)
    }

    fn get_transaction_by_hash(&self, params: Array) -> Result<JsonValue, JsonValue> {
        params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid transaction hash"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid transaction hash"}))
            .and_then(|h| self.get_transaction_by_hash_helper(&h))
    }

    fn get_transaction_receipt(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let hash = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid transaction hash"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid transaction hash"}))?;

        let transaction_info = self.consensus
            .blockchain.get_transaction_info_by_hash(&hash)
            .ok_or_else(|| object!{"message" => "Transaction not found"})?;

        // Get block which contains the transaction. If we don't find the block (for what reason?),
        // return an error
        let block = self.consensus.blockchain.get_block(&transaction_info.block_hash, false, true);

        let transaction_index = transaction_info.index;
        Ok(self.transaction_receipt_to_obj(&transaction_info.into(),
                                           Some(transaction_index),
                                           block.as_ref()))
    }

    fn get_transactions_by_address(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let address = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid address"})
            .and_then(|s| Address::from_any_str(s)
                .map_err(|_| object!{"message" => "Invalid address"}))?;

        // TODO: Accept two limit parameters?
        let limit = params.get(0).and_then(JsonValue::as_usize)
            .unwrap_or(1000);
        let sender_limit = limit / 2;
        let recipient_limit = limit / 2;

        Ok(JsonValue::Array(self.consensus.blockchain
            .get_transaction_receipts_by_address(&address, sender_limit, recipient_limit)
            .iter()
            .map(|receipt| self.transaction_receipt_to_obj(&receipt, None, None))
            .collect::<Array>()))
    }

    fn mempool_content(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let include_transactions = params.get(0).and_then(JsonValue::as_bool)
            .unwrap_or(false);

        Ok(JsonValue::Array(self.consensus.mempool.get_transactions(usize::max_value(), 0f64)
            .iter()
            .map(|tx| if include_transactions {
                self.transaction_to_obj(tx, None, None)
            } else {
                tx.hash::<Blake2bHash>().to_hex().into()
            })
            .collect::<Array>()))
    }

    fn mempool(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // Transactions sorted by fee/byte, ascending
        let transactions = self.consensus.mempool.get_transactions(usize::max_value(), 0f64);
        let bucket_values: [u64; 14] = [0, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000];
        let mut bucket_counts: [u32; 14] = [0; 14];
        let mut i = 0;

        for transaction in &transactions {
            while (transaction.fee_per_byte() as u64) < bucket_values[i] { i += 1; }
            bucket_counts[i] += 1;
        }

        let mut transactions_per_bucket = Object::new();
        let mut buckets = Array::new();
        transactions_per_bucket.insert("total", transactions.len().into());
        for (&value, &count) in bucket_values.iter().zip(&bucket_counts) {
            if count > 0 {
                transactions_per_bucket.insert(value.to_string().as_str(), JsonValue::from(count));
                buckets.push(value.into());
            }
        }
        transactions_per_bucket.insert("buckets", JsonValue::Array(buckets));

        Ok(JsonValue::Object(transactions_per_bucket))
    }

    // Blockchain

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
        Ok(self.block_to_obj(&self.block_by_hash(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }

    fn get_block_by_number(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_to_obj(&self.block_by_number(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }


    // Helper functions
    
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
    
    fn block_to_obj(&self, block: &Block, include_transactions: bool) -> JsonValue {
        object!{
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
                body.transactions.iter().enumerate().map(|(i, tx)| self.transaction_to_obj(tx, Some(block), Some(i))).collect()
            } else { 
                body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
            }).unwrap_or_else(Vec::new)),
        }
    }
    
    fn transaction_to_obj(&self, transaction: &Transaction, block: Option<&Block>, i: Option<usize>) -> JsonValue {
        let header = block.as_ref().map(|b| &b.header);
        object!{
            "hash" => transaction.hash::<Blake2bHash>().to_hex(),
            "blockHash" => header.map(|h| h.hash::<Blake2bHash>().to_hex().into()).unwrap_or(Null),
            "blockNumber" => header.map(|h| h.height.into()).unwrap_or(Null),
            "timestamp" => header.map(|h| h.timestamp.into()).unwrap_or(Null),
            "confirmations" => header.map(|b| (self.consensus.blockchain.height() - b.height).into()).unwrap_or(Null),
            "transactionIndex" => i.map(|i| i.into()).unwrap_or(Null),
            "from" => transaction.sender.to_hex(),
            "fromAddress" => transaction.sender.to_user_friendly_address(),
            "to" => transaction.recipient.to_hex(),
            "toAddress" => transaction.recipient.to_user_friendly_address(),
            "value" => u64::from(transaction.value),
            "fee" => u64::from(transaction.fee),
            "data" => hex::encode(&transaction.data),
            "flags" => transaction.flags.bits(),
            "validityStartHeight" => transaction.validity_start_height
        }
    }

    fn obj_to_transaction(&self, _obj: &JsonValue) -> Result<Transaction, JsonValue> {
        /*
        let from = Address::from_any_str(obj["from"].as_str()
            .ok_or_else(|| object!{"message" => "Sender address must be a string"})?)
            .map_err(|_|  object!{"message" => "Sender address invalid"})?;

        let from_type = match &obj["fromType"] {
            &JsonValue::Null => Some(AccountType::Basic),
            n @ JsonValue::Number(_) => n.as_u8().and_then(|n| AccountType::from_int(n)),
            _ => None
        }.ok_or_else(|| object!{"message" => "Invalid sender account type"})?;

        let to = Address::from_any_str(obj["to"].as_str()
            .ok_or_else(|| object!{"message" => "Recipient address must be a string"})?)
            .map_err(|_|  object!{"message" => "Recipient address invalid"})?;

        let to_type = match &obj["toType"] {
            &JsonValue::Null => Some(AccountType::Basic),
            n @ JsonValue::Number(_) => n.as_u8().and_then(|n| AccountType::from_int(n)),
            _ => None
        }.ok_or_else(|| object!{"message" => "Invalid recipient account type"})?;

        let value = Coin::from(obj["value"].as_u64()
            .ok_or_else(|| object!{"message" => "Invalid transaction value"})?);

        let fee = Coin::from(obj["value"].as_u64()
            .ok_or_else(|| object!{"message" => "Invalid transaction fee"})?);

        let flags = obj["flags"].as_u8()
            .map_or_else(|| Some(TransactionFlags::empty()), TransactionFlags::from_bits)
            .ok_or_else(|| object!{"message" => "Invalid transaction flags"})?;

        let data = obj["data"].as_str()
            .map(|d| hex::decode(d))
            .transpose().map_err(|_| object!{"message" => "Invalid transaction data"})?
            .unwrap_or(vec![]);
        */

        rpc_not_implemented()
    }

    fn push_transaction(&self, _transaction: &Transaction) -> Result<JsonValue, JsonValue> {
        rpc_not_implemented()
    }

    fn get_transaction_by_hash_helper(&self, hash: &Blake2bHash) -> Result<JsonValue, JsonValue> {
        // Get transaction info, which includes Block hash, transaction hash, and transaction index.
        // Return an error if the transaction doesn't exist.
        let transaction_info = self.consensus.blockchain.get_transaction_info_by_hash(hash)
            .ok_or_else(|| object!{"message" => "Transaction not found"})?;

        // Get block which contains the transaction. If we don't find the block (for what reason?),
        // return an error
        let block = self.consensus.blockchain.get_block(&transaction_info.block_hash, false, true)
            .ok_or_else(|| object!{"message" => "Block not found"})?;

        self.get_transaction_by_block_and_index(&block, transaction_info.index)
    }

    fn get_transaction_by_block_and_index(&self, block: &Block, index: u16) -> Result<JsonValue, JsonValue> {
        // Get the transaction. If the body doesn't store transaction, return an error
        let transaction = block.body.as_ref()
            .and_then(|b| b.transactions.get(index as usize))
            .ok_or_else(|| object!{"message" => "Block doesn't contain transaction."})?;

        Ok(self.transaction_to_obj(&transaction, Some(&block), Some(index as usize)))
    }

    fn peer_address_info_to_obj(&self, peer_address_info: &PeerAddressInfo, connection_info: Option<&ConnectionInfo>, score: Option<Score>) -> JsonValue {
        let state = self.consensus.network.connections.state();
        let connection_info = connection_info.or_else(|| {
            state.get_connection_by_peer_address(&peer_address_info.peer_address)
        });
        let peer = connection_info.and_then(|conn| conn.peer());

        object!{
            "id" => peer_address_info.peer_address.peer_id().to_hex(),
            "address" => peer_address_info.peer_address.as_uri().to_string(),
            "failedAttempts" => peer_address_info.failed_attempts,
            "addressState" => peer_address_info.state as u8,
            "connectionState" => connection_info.map(|conn| (conn.state() as u8).into()).unwrap_or(Null),
            "version" => peer.map(|peer| peer.version.into()).unwrap_or(Null),
            "timeOffset" => peer.map(|peer| peer.time_offset.into()).unwrap_or(Null),
            "headHash" => peer.map(|peer| peer.head_hash.to_hex().into()).unwrap_or(Null),
            "score" => score.map(|s| s.into()).unwrap_or(Null),
            "latency" => connection_info.map(|conn| conn.statistics().latency_median().into()).unwrap_or(Null),
            "rx" => Null, // TODO: Not in NetworkConnection
            "tx" => Null,
        }
    }

    fn transaction_receipt_to_obj(&self, receipt: &TransactionReceipt, index: Option<u16>, block: Option<&Block>) -> JsonValue {
        object!{
            "transactionHash" => receipt.transaction_hash.to_hex(),
            "blockNumber" => receipt.block_height,
            "blockHash" => receipt.block_hash.to_hex(),
            "confirmations" => self.consensus.blockchain.height() - receipt.block_height,
            "timestamp" => block.map(|block| block.header.timestamp.into()).unwrap_or(Null),
            "transactionIndex" => index.map(|i| i.into()).unwrap_or(Null)
        }
    }
}

impl jsonrpc::Handler for JsonRpcHandler {
    fn get_method(&self, name: &str) -> Option<fn(&Self, Array) -> Result<JsonValue, JsonValue>> {
        trace!("RPC method called: {}", name);

        if !self.config.methods.is_empty() && !self.config.methods.contains(name) {
            info!("RPC call to black-listed method: {}", name);
            //return Some(|_, _| Err(object!("message" => "Method is not allowed.")))
            return None
        }

        match name {
            // Network
            "peerCount" => Some(JsonRpcHandler::peer_count),
            "syncing" => Some(JsonRpcHandler::syncing),
            "consensus" => Some(JsonRpcHandler::consensus),
            "peerList" => Some(JsonRpcHandler::peer_list),
            "peerState" => Some(JsonRpcHandler::peer_state),

            // Transactions
            "sendRawTransaction" => Some(JsonRpcHandler::send_raw_transaction),
            "createRawTransaction" => Some(JsonRpcHandler::create_raw_transaction),
            "sendTransaction" => Some(JsonRpcHandler::send_transaction),
            "getRawTransactionInfo" => Some(JsonRpcHandler::get_raw_transaction_info),
            "getTransactionByBlockHashAndIndex" => Some(JsonRpcHandler::get_transaction_by_block_hash_and_index),
            "getTransactionByBlockNumberAndIndex" => Some(JsonRpcHandler::get_transaction_by_block_number_and_index),
            "getTransactionByHash" => Some(JsonRpcHandler::get_transaction_by_hash),
            "getTransactionReceipt" => Some(JsonRpcHandler::get_transaction_receipt),
            "getTransactionsByAddress" => Some(JsonRpcHandler::get_transactions_by_address),
            "mempoolContent" => Some(JsonRpcHandler::mempool_content),
            "mempool" => Some(JsonRpcHandler::mempool),

            // Blockchain
            "blockNumber" => Some(JsonRpcHandler::block_number),
            "getBlockTransactionCountByHash" => Some(JsonRpcHandler::get_block_transaction_count_by_hash),
            "getBlockTransactionCountByNumber" => Some(JsonRpcHandler::get_block_transaction_count_by_number),
            "getBlockByHash" => Some(JsonRpcHandler::get_block_by_hash),
            "getBlockByNumber" => Some(JsonRpcHandler::get_block_by_number),

            _ => None
        }
    }

    fn authorize(&self, username: &str, password: &str) -> Result<(), AuthenticationError> {
        if !self.config.credentials.as_ref().map(|c| c.check(username, password)).unwrap_or(true) {
            return Err(AuthenticationError::IncorrectCredentials);
        }
        Ok(())
    }
}


pub fn rpc_server(consensus: Arc<Consensus>, ip: IpAddr, port: u16, config: JsonRpcConfig) -> Result<Box<dyn Future<Item=(), Error=()> + Send + Sync>, Error> {
    let state = Arc::new(RwLock::new(JsonRpcServerState {
        consensus_state: "syncing",
    }));

    // Register for consensus events.
    {
        trace!("Register listener for consensus");
        let state = Arc::downgrade(&state);
        consensus.notifier.write().register(move |e: &ConsensusEvent| {
            trace!("Consensus Event: {:?}", e);
            if let Some(state) = state.upgrade() {
                match e {
                    ConsensusEvent::Established => { state.write().consensus_state = "established" },
                    ConsensusEvent::Lost => { state.write().consensus_state = "lost" },
                    ConsensusEvent::Syncing => { state.write().consensus_state = "syncing" },
                    _ => ()
                }
            }
        });
    }

    let config = Arc::new(config);
    Ok(Box::new(Server::try_bind(&SocketAddr::new(ip, port))?
        .serve(move || {
            jsonrpc::Service::new(JsonRpcHandler::new(Arc::clone(&consensus), Arc::clone(&state), Arc::clone(&config)))
        })
        .map_err(|e| error!("RPC server failed: {}", e)))) // as Box<dyn Future<Item=(), Error=()> + Send + Sync>
}
