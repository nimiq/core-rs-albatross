use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};
use parking_lot::RwLock;

use block_albatross::{Block, MicroBlock};
use consensus::{Consensus, AlbatrossConsensusProtocol};
use hash::{Blake2bHash, Hash};
use primitives::policy;

use crate::error::AuthenticationError;
use crate::jsonrpc;
use crate::{AbstractRpcHandler, JsonRpcConfig, JsonRpcServerState};
use crate::common::RpcHandler;

impl AbstractRpcHandler<AlbatrossConsensusProtocol> for RpcHandler<AlbatrossConsensusProtocol> {
    fn new(consensus: Arc<Consensus<AlbatrossConsensusProtocol>>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self {
        Self {
            state,
            consensus: consensus.clone(),
            starting_block: consensus.blockchain.height(),
            config
        }
    }
}

impl jsonrpc::Handler for RpcHandler<AlbatrossConsensusProtocol> {
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
            "getTransactionByBlockHashAndIndex" => Some(RpcHandler::<AlbatrossConsensusProtocol>::get_transaction_by_block_hash_and_index),
            "getTransactionByBlockNumberAndIndex" => Some(RpcHandler::<AlbatrossConsensusProtocol>::get_transaction_by_block_number_and_index),
            "mempoolContent" => Some(RpcHandler::mempool_content),
            "mempool" => Some(RpcHandler::mempool),

            // Blockchain
            "blockNumber" => Some(RpcHandler::block_number),
            "epochNumber" => Some(RpcHandler::epoch_number),
            "getBlockTransactionCountByHash" => Some(RpcHandler::<AlbatrossConsensusProtocol>::get_block_transaction_count_by_hash),
            "getBlockTransactionCountByNumber" => Some(RpcHandler::<AlbatrossConsensusProtocol>::get_block_transaction_count_by_number),
            "getBlockByHash" => Some(RpcHandler::<AlbatrossConsensusProtocol>::get_block_by_hash),
            "getBlockByNumber" => Some(RpcHandler::<AlbatrossConsensusProtocol>::get_block_by_number),

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

impl RpcHandler<AlbatrossConsensusProtocol> {

    // Blockchain

    pub(crate) fn epoch_number(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(policy::epoch_at(self.consensus.blockchain.height()).into())
    }

    fn get_block_transaction_count_by_hash(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_by_hash(params.get(0).unwrap_or(&Null))?
            .transactions().ok_or_else(|| object!{"message" => "No body or transactions for block found"})?
            .len().into())
    }

    fn get_block_transaction_count_by_number(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_by_number(params.get(0).unwrap_or(&Null))?
            .transactions().ok_or_else(|| object!{"message" => "No body or transactions for block found"})?
            .len().into())
    }

    fn get_block_by_hash(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_to_obj(&self.block_by_hash(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }

    fn get_block_by_number(&self, params: Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_to_obj(&self.block_by_number(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }

    // Transaction

    fn get_transaction_by_block_hash_and_index(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_hash(params.get(0).unwrap_or(&Null))?;
        let index = params.get(1).and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        if let Block::Micro(ref block) = block {
            self.get_transaction_by_block_and_index(&block, index)
        } else {
            Err(object!("message" => "Macro blocks don't contain transactions"))
        }
    }

    fn get_transaction_by_block_number_and_index(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_number(params.get(0).unwrap_or(&Null))?;
        let index = params.get(1).and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        if let Block::Micro(ref block) = block {
            self.get_transaction_by_block_and_index(&block, index)
        } else {
            Err(object!("message" => "Macro blocks don't contain transactions"))
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
        match block {
            Block::Macro(ref block) => object! {
                "type" => "macro",
                "blockNumber" => block.header.block_number,
                "viewNumber" => block.header.view_number,
                "parentMacroHash" => block.header.parent_macro_hash.to_hex(),
                "parentHash" => block.header.parent_hash.to_hex(),
                "seed" => hex::encode(&block.header.seed),
                "stateRoot" => block.header.state_root.to_hex(),
                "extrinsicsRoot" => block.header.extrinsics_root.to_hex(),
                "timestamp" => block.header.timestamp,
                // TODO Extrinsics
                // TODO Justification
            },
            Block::Micro(ref block) => object! {
                "type" => "micro",
                "blockNumber" => block.header.block_number,
                "viewNumber" => block.header.view_number,
                "parentHash" => block.header.parent_hash.to_hex(),
                "stateRoot" => block.header.state_root.to_hex(),
                "extrinsicsRoot" => block.header.extrinsics_root.to_hex(),
                "seed" => hex::encode(&block.header.seed),
                "timestamp" => block.header.timestamp,
                "extraData" => block.extrinsics.as_ref().map(|body| hex::encode(&body.extra_data).into()).unwrap_or(Null),
                // TODO Fork proofs
                "transactions" => JsonValue::Array(block.extrinsics.as_ref().map(|body| if include_transactions {
                    body.transactions.iter().enumerate().map(|(i, tx)| self.transaction_to_obj(tx, Some(block.header.block_number), Some(i))).collect()
                } else {
                    body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
                }).unwrap_or_else(Vec::new)),
                // TODO Receipts
                // TODO Justification
            }
        }
    }

    fn get_transaction_by_block_and_index(&self, block: &MicroBlock, index: u16) -> Result<JsonValue, JsonValue> {
        // Get the transaction. If the body doesn't store transaction, return an error
        let transaction = block.extrinsics.as_ref()
            .and_then(|b| b.transactions.get(index as usize))
            .ok_or_else(|| object!{"message" => "Block doesn't contain transaction."})?;

        Ok(self.transaction_to_obj(&transaction, Some(block.header.block_number), Some(index as usize)))
    }

}

