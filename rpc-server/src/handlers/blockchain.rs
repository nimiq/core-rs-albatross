use std::str::FromStr;
use std::sync::Arc;

use json::{object, Array, JsonValue, Null};

use block_base::{Block, BlockHeader};
use blockchain_base::AbstractBlockchain;
use keys::Address;

use nimiq_hash::Blake2bHash;
use nimiq_transaction::TransactionReceipt;

use crate::handlers::mempool::{transaction_to_obj, TransactionContext};

pub struct BlockchainHandler<B: AbstractBlockchain + 'static> {
    pub blockchain: Arc<B>,
}

impl<B: AbstractBlockchain + 'static> BlockchainHandler<B> {
    pub(crate) fn new(blockchain: Arc<B>) -> Self {
        Self { blockchain }
    }

    // Blocks

    /// Returns the current block number.
    pub(crate) fn block_number(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        Ok(self.blockchain.head_height().into())
    }

    /// Returns the number of transactions for a block hash.
    /// Parameters:
    /// - hash (string)
    pub(crate) fn get_block_transaction_count_by_hash(
        &self,
        params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
        Ok(self
            .block_by_hash(params.get(0).unwrap_or(&Null))?
            .transactions()
            .map(|l| l.len())
            .ok_or_else(|| object! {"message" => "No body for block found"})?
            .into())
    }

    /// Returns the number of transactions for a block number.
    /// Parameters:
    /// - height (number)
    pub(crate) fn get_block_transaction_count_by_number(
        &self,
        params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
        Ok(self
            .block_by_number(params.get(0).unwrap_or(&Null))?
            .transactions()
            .map(|l| l.len())
            .ok_or_else(|| object! {"message" => "No body for block found"})?
            .into())
    }

    // Transactions

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
    pub(crate) fn get_transaction_by_block_hash_and_index(
        &self,
        params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_hash(params.get(0).unwrap_or(&Null))?;
        let index = params
            .get(1)
            .and_then(JsonValue::as_u16)
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
    pub(crate) fn get_transaction_by_block_number_and_index(
        &self,
        params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
        let block = self.block_by_number(params.get(0).unwrap_or(&Null))?;
        let index = params
            .get(1)
            .and_then(JsonValue::as_u16)
            .ok_or_else(|| object!("message" => "Invalid transaction index"))?;
        self.get_transaction_by_block_and_index(&block, index)
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
    pub(crate) fn get_transactions_by_address(
        &self,
        params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
        let address = params
            .get(0)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid address"})
            .and_then(|s| {
                Address::from_any_str(s).map_err(|_| object! {"message" => "Invalid address"})
            })?;

        // TODO: Accept two limit parameters?
        let limit = params.get(0).and_then(JsonValue::as_usize).unwrap_or(1000);
        let sender_limit = limit / 2;
        let recipient_limit = limit / 2;

        Ok(JsonValue::Array(
            self.blockchain
                .get_transaction_receipts_by_address(&address, sender_limit, recipient_limit)
                .iter()
                .map(|receipt| self.transaction_receipt_to_obj(&receipt, None, None))
                .collect::<Array>(),
        ))
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
    pub(crate) fn get_balance(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let address = params
            .get(0)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid address"})
            .and_then(|s| {
                Address::from_any_str(s).map_err(|_| object! {"message" => "Invalid address"})
            })?;

        let account = self.blockchain.get_account(&address);
        Ok(JsonValue::from(u64::from(account.balance())))
    }

    // Helper functions

    pub(crate) fn block_by_number(&self, number: &JsonValue) -> Result<B::Block, JsonValue> {
        let block_number = self.parse_block_number(number)?;
        self.blockchain
            .get_block_at(block_number, true)
            .ok_or_else(|| object! {"message" => "Block not found"})
    }

    pub(crate) fn block_by_hash(&self, hash: &JsonValue) -> Result<B::Block, JsonValue> {
        let hash = parse_hash(hash)?;
        self.blockchain
            .get_block(&hash, true)
            .ok_or_else(|| object! {"message" => "Block not found"})
    }

    pub(crate) fn get_transaction_by_block_and_index(
        &self,
        block: &B::Block,
        index: u16,
    ) -> Result<JsonValue, JsonValue> {
        // Get the transaction. If the body doesn't store transaction, return an error
        let transaction = block
            .transactions()
            .and_then(|l| l.get(index as usize))
            .ok_or_else(|| object! {"message" => "Block doesn't contain transaction."})?;

        Ok(transaction_to_obj(
            &transaction,
            Some(&TransactionContext {
                block_hash: &block.hash().to_hex(),
                block_number: block.height(),
                index,
                timestamp: block.header().timestamp(),
            }),
            Some(self.blockchain.head_height()),
        ))
    }

    pub(crate) fn transaction_receipt_to_obj(
        &self,
        receipt: &TransactionReceipt,
        index: Option<u16>,
        block: Option<&B::Block>,
    ) -> JsonValue {
        object! {
            "transactionHash" => receipt.transaction_hash.to_hex(),
            "blockNumber" => receipt.block_height,
            "blockHash" => receipt.block_hash.to_hex(),
            "confirmations" => self.blockchain.head_height() - receipt.block_height,
            "timestamp" => block.map(|block| block.header().timestamp().into()).unwrap_or(Null),
            "timestampMillis" => block.map(|block| (block.header().timestamp() / 1000).into()).unwrap_or(Null),
            "transactionIndex" => index.map(|i| i.into()).unwrap_or(Null)
        }
    }

    pub(crate) fn parse_block_number(&self, number: &JsonValue) -> Result<u32, JsonValue> {
        let block_number = if number.is_string() {
            if number.as_str().unwrap().starts_with("latest-") {
                let offset = u32::from_str(&number.as_str().unwrap()[7..])
                    .map_err(|_| object! {"message" => "Invalid block number"})?;
                self.blockchain.head_height() - offset
            } else if number.as_str().unwrap() == "latest" {
                self.blockchain.head_height()
            } else {
                u32::from_str(number.as_str().unwrap())
                    .map_err(|_| object! {"message" => "Invalid block number"})?
            }
        } else if number.is_number() {
            number
                .as_u32()
                .ok_or_else(|| object! {"message" => "Invalid block number"})?
        } else {
            return Err(object! {"message" => "Invalid block number"});
        };
        Ok(block_number)
    }
}

pub(crate) fn parse_hash(hash: &JsonValue) -> Result<Blake2bHash, JsonValue> {
    hash.as_str()
        .ok_or_else(|| object! {"message" => "Hash must be a string"})
        .and_then(|s| {
            Blake2bHash::from_str(s).map_err(|_| object! {"message" => "Invalid Blake2b hash"})
        })
}
