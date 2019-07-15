use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};

use beserial::{Deserialize, Serialize};
use block::Block;
use block::Difficulty;
use blockchain_base::AbstractBlockchain;
use hash::{Argon2dHash, Blake2bHash, Hash};
use keys::Address;
use nimiq_blockchain::Blockchain;
use transaction::{Transaction, TransactionReceipt};

use crate::handlers::Handler;
use crate::handlers::mempool::{transaction_to_obj, TransactionContext};
use crate::rpc_not_implemented;

pub struct BlockchainHandler {
    pub blockchain: Arc<Blockchain<'static>>,
}

impl BlockchainHandler {
    pub(crate) fn new(blockchain: Arc<Blockchain<'static>>) -> Self {
        BlockchainHandler {
            blockchain,
        }
    }

    // Blocks
    /// Returns the current block number.
    pub(crate) fn block_number(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(self.blockchain.head_height().into())
    }

    /// Returns the number of transactions for a block hash.
    /// Parameters:
    /// - hash (string)
    pub(crate) fn get_block_transaction_count_by_hash(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_by_hash(params.get(0).unwrap_or(&Null))?
            .body.ok_or_else(|| object!{"message" => "No body for block found"})?
            .transactions.len().into())
    }

    /// Returns the number of transactions for a block number.
    /// Parameters:
    /// - height (number)
    pub(crate) fn get_block_transaction_count_by_number(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(self.block_by_number(params.get(0).unwrap_or(&Null))?
            .body.ok_or_else(|| object!{"message" => "No body for block found"})?
            .transactions.len().into())
    }

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
        Ok(self.block_to_obj(&self.block_by_hash(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
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
        Ok(self.block_to_obj(&self.block_by_number(params.get(0).unwrap_or(&Null))?, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
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

    /// Retrieves information about a transaction by its block hash and transaction index.
    /// Parameters:
    /// - blockHash (string)
    /// - transactionIndex (number)
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
    pub(crate) fn get_transaction_by_block_hash_and_index(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_hash(params.get(0).unwrap_or(&Null))?;
        let index = params.get(1).and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        self.get_transaction_by_block_and_index(&block, index)
    }

    /// Retrieves information about a transaction by its block number and transaction index.
    /// Parameters:
    /// - blockNumber (number)
    /// - transactionIndex (number)
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
    pub(crate) fn get_transaction_by_block_number_and_index(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_number(params.get(0).unwrap_or(&Null))?;
        let index = params.get(1).and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        self.get_transaction_by_block_and_index(&block, index)
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
        params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid transaction hash"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid transaction hash"}))
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

    /// Retrieves transaction receipts for an address.
    /// Parameters:
    /// - address (string)
    ///
    /// Returns a list of receipts:
    /// ```text
    /// Array<{
    ///     transactionHash: string,
    ///     blockHash: string,
    ///     blockNumber: number,
    ///     timestamp: number,
    ///     confirmations: number,
    ///     transactionIndex: number,
    /// }>
    /// ```
    pub(crate) fn get_transactions_by_address(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let address = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid address"})
            .and_then(|s| Address::from_any_str(s)
                .map_err(|_| object!{"message" => "Invalid address"}))?;

        // TODO: Accept two limit parameters?
        let limit = params.get(0).and_then(JsonValue::as_usize)
            .unwrap_or(1000);
        let sender_limit = limit / 2;
        let recipient_limit = limit / 2;

        Ok(JsonValue::Array(self.blockchain
            .get_transaction_receipts_by_address(&address, sender_limit, recipient_limit)
            .iter()
            .map(|receipt| self.transaction_receipt_to_obj(&receipt, None, None))
            .collect::<Array>()))
    }

    // Accounts

    /// Look up the balance of an address.
    /// Parameters:
    /// - address (string)
    ///
    /// Returns the amount in Luna (10000 = 1 NIM):
    /// ```text
    /// 1200000
    /// ```
    pub(crate) fn get_balance(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let address = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid address"})
            .and_then(|s| Address::from_any_str(s)
                .map_err(|_| object!{"message" => "Invalid address"}))?;

        let account = self.blockchain.get_account(&address);
        Ok(JsonValue::from(u64::from(account.balance())))
    }

    // Helper functions

    fn block_by_number(&self, number: &JsonValue) -> Result<Block, JsonValue> {
        let mut block_number = if number.is_string() {
            if number.as_str().unwrap().starts_with("latest-") {
                self.blockchain.height() - u32::from_str(&number.as_str().unwrap()[7..]).map_err(|_| object!{"message" => "Invalid block number"})?
            } else if number.as_str().unwrap() == "latest" {
                self.blockchain.height()
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
        self.blockchain
            .get_block_at(block_number, true)
            .ok_or_else(|| object!{"message" => "Block not found"})
    }

    fn block_by_hash(&self, hash: &JsonValue) -> Result<Block, JsonValue> {
        let hash = hash.as_str()
            .ok_or_else(|| object!{"message" => "Hash must be a string"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid Blake2b hash"}))?;
        self.blockchain.get_block(&hash, false, true)
            .ok_or_else(|| object!{"message" => "Block not found"})
    }

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
                    timestamp: u64::from(block.header.timestamp),
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

        self.get_transaction_by_block_and_index(&block, transaction_info.index)
    }

    fn get_transaction_by_block_and_index(&self, block: &Block, index: u16) -> Result<JsonValue, JsonValue> {
        // Get the transaction. If the body doesn't store transaction, return an error
        let transaction = block.body.as_ref()
            .and_then(|b| b.transactions.get(index as usize))
            .ok_or_else(|| object!{"message" => "Block doesn't contain transaction."})?;

        Ok(transaction_to_obj(&transaction, Some(&TransactionContext {
            block_hash: &block.header.hash::<Blake2bHash>().to_hex(),
            block_number: block.header.height,
            index,
            timestamp: u64::from(block.header.timestamp),
        }), Some(self.blockchain.height())))
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

impl Handler for BlockchainHandler {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>> {
        match name {
            // Transactions
            "getRawTransactionInfo" => Some(self.get_raw_transaction_info(params)),
            "getTransactionByBlockHashAndIndex" => Some(self.get_transaction_by_block_hash_and_index(params)),
            "getTransactionByBlockNumberAndIndex" => Some(self.get_transaction_by_block_number_and_index(params)),
            "getTransactionByHash" => Some(self.get_transaction_by_hash(params)),
            "getTransactionReceipt" => Some(self.get_transaction_receipt(params)),
            "getTransactionsByAddress" => Some(self.get_transactions_by_address(params)),

            // Blockchain
            "blockNumber" => Some(self.block_number(params)),
            "getBlockTransactionCountByHash" => Some(self.get_block_transaction_count_by_hash(params)),
            "getBlockTransactionCountByNumber" => Some(self.get_block_transaction_count_by_number(params)),
            "getBlockByHash" => Some(self.get_block_by_hash(params)),
            "getBlockByNumber" => Some(self.get_block_by_number(params)),

            // Accounts
            "getBalance" => Some(self.get_balance(params)),

            _ => None
        }
    }
}
