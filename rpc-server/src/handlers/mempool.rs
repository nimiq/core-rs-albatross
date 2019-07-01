use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};
use json::object::Object;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use consensus::ConsensusProtocol;
use hash::{Blake2bHash, Hash};
use nimiq_mempool::Mempool;
use nimiq_mempool::ReturnCode;
use transaction::Transaction;

use crate::handlers::Handler;
use crate::JsonRpcServerState;
use crate::rpc_not_implemented;

pub struct MempoolHandler<P: ConsensusProtocol + 'static> {
    pub mempool: Arc<Mempool<'static, P::Blockchain>>,
}

impl<P: ConsensusProtocol + 'static> MempoolHandler<P> {
    pub(crate) fn new(mempool: Arc<Mempool<'static, P::Blockchain>>) -> Self {
        MempoolHandler {
            mempool,
        }
    }

    /// Returns the mempool content as an array.
    /// Parameters:
    /// - includeTransactions (bool, optional): Default is `false`.
    ///     If set to `true`, the full transactions will be included,
    ///     otherwise it will only include the hashes.
    pub(crate) fn mempool_content(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let include_transactions = params.get(0).and_then(JsonValue::as_bool)
            .unwrap_or(false);

        Ok(JsonValue::Array(self.mempool.get_transactions(usize::max_value(), 0f64)
            .iter()
            .map(|tx| if include_transactions {
                transaction_to_obj(tx, None, None)
            } else {
                tx.hash::<Blake2bHash>().to_hex().into()
            })
            .collect::<Array>()))
    }

    /// Returns mempool statistics on the number of transactions ordered by fee/byte.
    /// The numbers will be reported for the buckets `[0, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000]`.
    /// {
    ///     total: number,
    ///     buckets: Array<number>,
    /// }
    pub(crate) fn mempool(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        // Transactions sorted by fee/byte, ascending
        let transactions = self.mempool.get_transactions(usize::max_value(), 0f64);
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

    /// Sends a raw transaction.
    /// Parameters:
    /// - transaction (string)
    pub(crate) fn send_raw_transaction(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let raw = hex::decode(params.get(0)
            .unwrap_or(&Null)
            .as_str()
            .ok_or_else(|| object!{"message" => "Raw transaction must be a string"} )?)
            .map_err(|_| object!{"message" => "Raw transaction must be a hex string"} )?;
        let transaction: Transaction = Deserialize::deserialize_from_vec(&raw)
            .map_err(|_| object!{"message" => "Transaction can't be deserialized"} )?;
        self.push_transaction(transaction)
    }

    /// Returns a raw transaction (hex encoded string) from a transaction object.
    /// Parameters:
    /// - transaction (object)
    ///
    /// The transaction looks like the following:
    /// {
    ///     from: string,
    ///     fromType: number|null,
    ///     to: string,
    ///     toType: number|null,
    ///     value: number, (in Luna)
    ///     fee: number, (in Luna)
    ///     flags: number|null,
    ///     data: string|null,
    ///     validityStartHeight: number|null,
    /// }
    /// Fields that can be null are optional.
    pub(crate) fn create_raw_transaction(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let raw = Serialize::serialize_to_vec(&obj_to_transaction(params.get(0).unwrap_or(&Null))?);
        Ok(hex::encode(raw).into())
    }

    /// Creates and sends a transaction from a transaction object.
    /// Parameters:
    /// - transaction (object)
    ///
    /// The transaction looks like the following:
    /// {
    ///     from: string,
    ///     fromType: number|null,
    ///     to: string,
    ///     toType: number|null,
    ///     value: number, (in Luna)
    ///     fee: number, (in Luna)
    ///     flags: number|null,
    ///     data: string|null,
    ///     validityStartHeight: number|null,
    /// }
    /// Fields that can be null are optional.
    pub(crate) fn send_transaction(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        self.push_transaction(obj_to_transaction(params.get(0).unwrap_or(&Null))?)
    }

    /// Returns the transaction for a hash if it is in the mempool and `null` otherwise.
    /// Parameters:
    /// - transactionHash (string)
    ///
    /// The transaction object returned has the following fields:
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
    ///     // Not applicable:
    ///     blockHash: null,
    ///     blockNumber: null,
    ///     timestamp: null,
    ///     confirmations: null,
    ///     transactionIndex: null,
    /// }
    pub(crate) fn get_transaction(&self, params: &Array) -> Result<JsonValue, JsonValue> {
        let hash = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object!{"message" => "Invalid transaction hash"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object!{"message" => "Invalid transaction hash"}))?;
        let transaction = self.mempool.get_transaction(&hash);
        if let Some(tx) = transaction {
            Ok(transaction_to_obj(&tx, None, None))
        } else {
            Ok(Null)
        }
    }

    // Helper functions

    fn push_transaction(&self, transaction: Transaction) -> Result<JsonValue, JsonValue> {
        match self.mempool.push_transaction(transaction) {
            ReturnCode::Accepted | ReturnCode::Known => Ok(object!{"message" => "Ok"}),
            code => Err(object!{"message" => format!("Rejected: {:?}", code)})
        }
    }
}

pub(crate) fn transaction_to_obj(transaction: &Transaction, context: Option<&TransactionContext>, head_height: Option<u32>) -> JsonValue {
    object!{
            "hash" => transaction.hash::<Blake2bHash>().to_hex(),
            "blockHash" => context.map(|c| c.block_hash.into()).unwrap_or(Null),
            "blockNumber" => context.map(|c| c.block_number.into()).unwrap_or(Null),
            "timestamp" => context.map(|c| c.timestamp.into()).unwrap_or(Null),
            "confirmations" => context.map(|c| head_height.map(|height| (height - c.block_number).into()).unwrap_or(Null)).unwrap_or(Null),
            "transactionIndex" => context.map(|c| c.index.into()).unwrap_or(Null),
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

pub(crate) fn obj_to_transaction(_obj: &JsonValue) -> Result<Transaction, JsonValue> {
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

pub(crate) struct TransactionContext<'a> {
    pub block_hash: &'a str,
    pub block_number: u32,
    pub index: u16,
    pub timestamp: u64,
}

impl<P: ConsensusProtocol + 'static> Handler for MempoolHandler<P> {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>> {
        match name {
            // Transactions
            "sendRawTransaction" => Some(self.send_raw_transaction(params)),
            "createRawTransaction" => Some(self.create_raw_transaction(params)),
            "sendTransaction" => Some(self.send_transaction(params)),
            "mempoolContent" => Some(self.mempool_content(params)),
            "mempool" => Some(self.mempool(params)),

            _ => None
        }
    }
}
