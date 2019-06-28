use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use hex;
use json::{Array, JsonValue, Null};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use block::{Block, BlockHeader, Difficulty};
use block_production::BlockProducer;
use blockchain::PushResult;
use consensus::{Consensus, NimiqConsensusProtocol};
use hash::{Argon2dHash, Blake2bHash, Blake2bHasher, Hash};
use keys::Address;
use transaction::{Transaction, TransactionReceipt};
use utils::merkle::MerklePath;
use utils::time::systemtime_to_timestamp;

use crate::error::AuthenticationError;
use crate::{jsonrpc, rpc_not_implemented};
use crate::{AbstractRpcHandler, JsonRpcConfig, JsonRpcServerState};
use crate::common::{RpcHandler, TransactionContext};

impl AbstractRpcHandler<NimiqConsensusProtocol> for RpcHandler<NimiqConsensusProtocol> {
    fn new(consensus: Arc<Consensus<NimiqConsensusProtocol>>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self {
        Self {
            state,
            starting_block: consensus.blockchain.height(),
            consensus,
            config
        }
    }
}

impl jsonrpc::Handler for RpcHandler<NimiqConsensusProtocol> {
    fn get_method(&self, name: &str) -> Option<fn(&Self, Array) -> Result<JsonValue, JsonValue>> {
        trace!("RPC method called: {}", name);

        if !self.config.methods.is_empty() && !self.config.methods.contains(name) {
            info!("RPC call to black-listed method: {}", name);
            //return Some(|_, _| Err(object!("message" => "Method is not allowed.")))
            return None
        }

        match name {
            // Network
            "peerCount" => Some(RpcHandler::peer_count),
            "syncing" => Some(RpcHandler::syncing),
            "consensus" => Some(RpcHandler::consensus),
            "peerList" => Some(RpcHandler::peer_list),
            "peerState" => Some(RpcHandler::peer_state),

            // Transactions
            "sendRawTransaction" => Some(RpcHandler::send_raw_transaction),
            "createRawTransaction" => Some(RpcHandler::create_raw_transaction),
            "sendTransaction" => Some(RpcHandler::send_transaction),
            "getRawTransactionInfo" => Some(RpcHandler::<NimiqConsensusProtocol>::get_raw_transaction_info),
            "getTransactionByBlockHashAndIndex" => Some(RpcHandler::<NimiqConsensusProtocol>::get_transaction_by_block_hash_and_index),
            "getTransactionByBlockNumberAndIndex" => Some(RpcHandler::<NimiqConsensusProtocol>::get_transaction_by_block_number_and_index),
            "getTransactionByHash" => Some(RpcHandler::<NimiqConsensusProtocol>::get_transaction_by_hash),
            "getTransactionReceipt" => Some(RpcHandler::<NimiqConsensusProtocol>::get_transaction_receipt),
            "getTransactionsByAddress" => Some(RpcHandler::<NimiqConsensusProtocol>::get_transactions_by_address),
            "mempoolContent" => Some(RpcHandler::mempool_content),
            "mempool" => Some(RpcHandler::mempool),

            // Blockchain
            "blockNumber" => Some(RpcHandler::block_number),
            "getBlockTransactionCountByHash" => Some(RpcHandler::<NimiqConsensusProtocol>::get_block_transaction_count_by_hash),
            "getBlockTransactionCountByNumber" => Some(RpcHandler::<NimiqConsensusProtocol>::get_block_transaction_count_by_number),
            "getBlockByHash" => Some(RpcHandler::<NimiqConsensusProtocol>::get_block_by_hash),
            "getBlockByNumber" => Some(RpcHandler::<NimiqConsensusProtocol>::get_block_by_number),

            // Block production
            "getWork" => Some(RpcHandler::<NimiqConsensusProtocol>::get_work),
            "getBlockTemplate" => Some(RpcHandler::<NimiqConsensusProtocol>::get_block_template),
            "submitBlock" => Some(RpcHandler::<NimiqConsensusProtocol>::submit_block),

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

impl RpcHandler<NimiqConsensusProtocol> {

    // Transaction

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
                (self.transaction_to_obj(&transaction, None),
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



    // Blockchain

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


    // Block production

    fn get_work(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = self.produce_block(params)?;
        let block_bytes = block.serialize_to_vec();

        Ok(object!{
            "data" => hex::encode(&block_bytes[..BlockHeader::SIZE]),
            "suffix" => hex::encode(&block_bytes[BlockHeader::SIZE..]),
            "target" => u32::from(block.header.n_bits),
            "algorithm" => "nimiq-argon2",
        })
    }

    fn get_block_template(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = self.produce_block(params)?;
        let header = block.header;
        let json_header = object!{
            "version" => header.version,
            "prevHash" => header.prev_hash.to_hex(),
            "interlinkHash" => header.interlink_hash.to_hex(),
            "accountsHash" => header.accounts_hash.to_hex(),
            "nBits" => u32::from(header.n_bits),
            "height" => header.height,
        };
        let body = block.body.unwrap();
        let merkle_path = MerklePath::new::<Blake2bHasher, Blake2bHash>(&body.get_merkle_leaves::<Blake2bHash>(), &body.miner.hash::<Blake2bHash>());
        let json_body = object!{
            "hash" => header.body_hash.to_hex(),
            "minerAddr" => body.miner.to_hex(),
            "extraData" => hex::encode(body.extra_data),
            "transactions" => JsonValue::Array(body.transactions.iter().map(|tx| self.transaction_to_obj(tx, None)).collect()),
            "merkleHashes" => JsonValue::Array(merkle_path.hashes().iter().map(|hash| JsonValue::String(hash.to_hex())).skip(1).collect()),
            "prunedAccounts" => JsonValue::Array(body.receipts.iter().map(|acc| JsonValue::String(hex::encode(acc.serialize_to_vec()))).collect()),
        };

        Ok(object!{
            "header" => json_header,
            "interlink" => hex::encode(block.interlink.serialize_to_vec()),
            "target" => u32::from(header.n_bits),
            "body" => json_body,
        })
    }

    fn submit_block(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Block must be a string"})
            .and_then(|s| hex::decode(s)
                .map_err(|_| object!{"message" => "Block must be hex-encoded"}))
            .and_then(|b| Block::deserialize_from_vec(&b)
                .map_err(|_| object!{"message" => "Invalid block data"}))?;

        match self.consensus.blockchain.push(block) {
            Ok(PushResult::Forked) => Ok(object!{"message" => "Forked"}),
            Ok(_) => Ok(object!{"message" => "Ok"}),
            _ => Err(object!{"message" => "Block rejected"})
        }
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
        let hash = block.header.hash::<Blake2bHash>().to_hex();
        object!{
            "number" => block.header.height,
            "hash" => hash.clone(),
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
                body.transactions.iter().enumerate().map(|(i, tx)| self.transaction_to_obj(tx, Some(&TransactionContext {
                    block_hash: &hash,
                    block_number: block.header.height,
                    index: i as u16,
                    timestamp: block.header.timestamp as u64,
                }))).collect()
            } else {
                body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
            }).unwrap_or_else(Vec::new)),
        }
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

        Ok(self.transaction_to_obj(&transaction, Some(&TransactionContext {
            block_hash: &block.header.hash::<Blake2bHash>().to_hex(),
            block_number: block.header.height,
            index,
            timestamp: block.header.timestamp as u64,
        })))
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

    fn produce_block(&self, params: Array) -> Result<Block, JsonValue> {
        let miner = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Miner address must be a string"})
            .and_then(|s| Address::from_any_str(s)
                .map_err(|_| object!{"message" => "Invalid miner address"}))?;

        let extra_data = params.get(1).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Extra data must be a string"})
            .and_then(|s| hex::decode(s)
                .map_err(|_| object!{"message" => "Extra data must be hex-encoded"}))?;

        let timestamp = (systemtime_to_timestamp(SystemTime::now()) / 1000) as u32;

        let producer = BlockProducer::new(self.consensus.blockchain.clone(), self.consensus.mempool.clone());
        return Ok(producer.next_block(timestamp, miner, extra_data));
    }
}

