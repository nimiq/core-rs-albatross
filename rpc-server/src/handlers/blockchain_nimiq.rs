use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};

use beserial::{Deserialize, Serialize};
use block::Block;
use block::Difficulty;
use hash::{Argon2dHash, Blake2bHash, Hash};
use nimiq_blockchain::Blockchain;
use transaction::{Transaction, TransactionReceipt};

use crate::handlers::Handler;
use crate::handlers::blockchain::{parse_hash, BlockchainHandler};
use crate::handlers::mempool::{transaction_to_obj, TransactionContext};
use crate::rpc_not_implemented;

pub struct BlockchainNimiqHandler {
    pub blockchain: Arc<Blockchain<'static>>,
    generic: BlockchainHandler<Blockchain<'static>>,
}

impl BlockchainNimiqHandler {
    pub(crate) fn new(blockchain: Arc<Blockchain<'static>>) -> Self {
        Self {
            generic: BlockchainHandler::new(blockchain.clone()),
            blockchain,
        }
    }

    // Blocks

    /// Returns a block object for a block hash.
    /// Parameters:
    /// - hash (string)
    /// - includeTransactions (bool, optional): Default is false. If set to false, only hashes are included.
    ///
    /// The block object contains:
    /// ```text
    /// {
    ///     number: number,
    ///     hash: string,
    ///     pow: string,
    ///     parentHash: string,
    ///     nonce: number,
    ///     bodyHash: string,
    ///     accountsHash: string,
    ///     miner: string,
    ///     minerAddress: string, (user friendly address)
    ///     difficulty: string,
    ///     extraData: string,
    ///     size: number,
    ///     timestamp: number,
    ///     transactions: Array<transaction_objects> | Array<string>, (depends on includeTransactions),
    /// }
    /// ```
    pub(crate) fn get_block_by_hash(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let block = self.generic.block_by_hash(params.get(0).unwrap_or(&Null))?;
        Ok(self.block_to_obj(&block, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }

    /// Returns a block object for a block number.
    /// Parameters:
    /// - height (number)
    /// - includeTransactions (bool, optional): Default is false. If set to false, only hashes are included.
    ///
    /// The block object contains:
    /// ```text
    /// {
    ///     number: number,
    ///     hash: string,
    ///     pow: string,
    ///     parentHash: string,
    ///     nonce: number,
    ///     bodyHash: string,
    ///     accountsHash: string,
    ///     miner: string,
    ///     minerAddress: string, (user friendly address)
    ///     difficulty: string,
    ///     extraData: string,
    ///     size: number,
    ///     timestamp: number,
    ///     transactions: Array<transaction_objects> | Array<string>, (depends on includeTransactions),
    /// }
    /// ```
    pub(crate) fn get_block_by_number(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let block = self.generic.block_by_number(params.get(0).unwrap_or(&Null))?;
        Ok(self.block_to_obj(&block, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }

    // Transactions

    /// Retrieves information about a transaction from its hex encoded form.
    /// Parameters:
    /// - transaction (string): Hex encoded transaction.
    ///
    /// Returns an info object:
    pub(crate) fn get_raw_transaction_info(&self, params: &Array) -> Result<JsonValue, JsonValue> {
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
                (transaction_to_obj(&transaction, None, None),
                 transaction.verify(self.blockchain.network_id).is_ok(), false)
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

    /// Retrieves information about a transaction by its hash.
    /// Parameters:
    /// - transactionHash (string)
    ///
    /// Returns an info object:
    /// ```text
    /// {
    ///     hash: string,
    ///     from: string, // hex encoded
    ///     fromAddress: string, // user friendly address
    ///     fromType: number,
    ///     to: string, // hex encoded
    ///     toAddress: string, // user friendly address
    ///     toType: number,
    ///     value: number,
    ///     fee: number,
    ///     data: string,
    ///     flags: number,
    ///     validityStartHeight: number,
    ///
    ///     blockHash: string,
    ///     blockNumber: number,
    ///     timestamp: number,
    ///     confirmations: number,
    ///     transactionIndex: number,
    /// }
    /// ```
    pub(crate) fn get_transaction_by_hash(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        params.get(0)
            .ok_or(object!{"message" => "First argument must be hash"})
            .and_then(parse_hash)
            .and_then(|h| self.get_transaction_by_hash_helper(&h))
    }

    /// Retrieves a transaction receipt by its hash.
    /// Parameters:
    /// - transactionHash (string)
    ///
    /// Returns a receipt:
    /// ```text
    /// {
    ///     transactionHash: string,
    ///     blockHash: string,
    ///     blockNumber: number,
    ///     timestamp: number,
    ///     confirmations: number,
    ///     transactionIndex: number,
    /// }
    /// ```
    pub(crate) fn get_transaction_receipt(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let hash = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid transaction hash"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid transaction hash"}))?;

        let transaction_info = self.blockchain.get_transaction_info_by_hash(&hash)
            .ok_or_else(|| object!{"message" => "Transaction not found"})?;

        // Get block which contains the transaction. If we don't find the block (for what reason?),
        // return an error
        let block = self.blockchain.get_block(&transaction_info.block_hash, false, true);

        let transaction_index = transaction_info.index;
        Ok(self.transaction_receipt_to_obj(&transaction_info.into(),
                                           Some(transaction_index),
                                           block.as_ref()))
    }

    // Helper functions

    fn block_to_obj(&self, block: &Block, include_transactions: bool) -> JsonValue {
        let hash = block.header.hash::<Blake2bHash>().to_hex();
        let height = self.blockchain.height();
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
                body.transactions.iter().enumerate().map(|(i, tx)| transaction_to_obj(tx, Some(&TransactionContext {
                    block_hash: &hash,
                    block_number: block.header.height,
                    index: i as u16,
                    //timestamp: block.header.timestamp as u64,
                }), Some(height))).collect()
            } else {
                body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
            }).unwrap_or_else(Vec::new)),
        }
    }

    fn get_transaction_by_hash_helper(&self, hash: &Blake2bHash) -> Result<JsonValue, JsonValue> {
        // Get transaction info, which includes Block hash, transaction hash, and transaction index.
        // Return an error if the transaction doesn't exist.
        let transaction_info = self.blockchain.get_transaction_info_by_hash(hash)
            .ok_or_else(|| object!{"message" => "Transaction not found"})?;

        // Get block which contains the transaction. If we don't find the block (for what reason?),
        // return an error
        let block = self.blockchain.get_block(&transaction_info.block_hash, false, true)
            .ok_or_else(|| object!{"message" => "Block not found"})?;

        self.generic.get_transaction_by_block_and_index(&block, transaction_info.index)
    }

    fn transaction_receipt_to_obj(&self, receipt: &TransactionReceipt, index: Option<u16>, block: Option<&Block>) -> JsonValue {
        object!{
            "transactionHash" => receipt.transaction_hash.to_hex(),
            "blockNumber" => receipt.block_height,
            "blockHash" => receipt.block_hash.to_hex(),
            "confirmations" => self.blockchain.height() - receipt.block_height,
            "timestamp" => block.map(|block| block.header.timestamp.into()).unwrap_or(Null),
            "transactionIndex" => index.map(|i| i.into()).unwrap_or(Null)
        }
    }
}

impl Handler for BlockchainNimiqHandler {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>> {
        if let Some(res) = self.generic.call(name, params) {
            return Some(res);
        }

        match name {
            // Transactions
            "getRawTransactionInfo" => Some(self.get_raw_transaction_info(params)),
            "getTransactionByHash" => Some(self.get_transaction_by_hash(params)),
            "getTransactionReceipt" => Some(self.get_transaction_receipt(params)),

            // Blockchain
            "getBlockByHash" => Some(self.get_block_by_hash(params)),
            "getBlockByNumber" => Some(self.get_block_by_number(params)),

            _ => None
        }
    }
}
