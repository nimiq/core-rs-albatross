use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};
use json::object::Object;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use consensus::ConsensusProtocol;
use hash::{Blake2bHash, Hash};
use keys::Address;
use nimiq_mempool::Mempool;
use nimiq_mempool::ReturnCode;
use primitives::account::AccountType;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::{Transaction, TransactionFlags};

use crate::handler::Method;
use crate::handlers::Module;
use crate::handlers::wallet::UnlockedWalletManager;

pub struct MempoolHandler<P: ConsensusProtocol + 'static> {
    pub mempool: Arc<Mempool<P::Blockchain>>,
    pub unlocked_wallets: Option<Arc<RwLock<UnlockedWalletManager>>>,
}

impl<P: ConsensusProtocol + 'static> MempoolHandler<P> {
    pub fn new(mempool: Arc<Mempool<P::Blockchain>>, unlocked_wallets: Option<Arc<RwLock<UnlockedWalletManager>>>) -> Self {
        MempoolHandler {
            mempool,
            unlocked_wallets,
        }
    }

    /// Returns the mempool content as an array.
    /// Parameters:
    /// - includeTransactions (bool, optional): Default is `false`.
    ///     If set to `true`, the full transactions will be included,
    ///     otherwise it will only include the hashes.
    pub(crate) fn mempool_content(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
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
    /// ```text
    /// {
    ///     total: number,
    ///     buckets: Array<number>,
    /// }
    /// ```
    pub(crate) fn mempool(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
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
    pub(crate) fn send_raw_transaction(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let raw = hex::decode(params.get(0)
            .unwrap_or(&Null)
            .as_str()
            .ok_or_else(|| object! {"message" => "Raw transaction must be a string"} )?)
            .map_err(|_| object! {"message" => "Raw transaction must be a hex string"} )?;
        let transaction: Transaction = Deserialize::deserialize_from_vec(&raw)
            .map_err(|_| object! {"message" => "Transaction can't be deserialized"} )?;
        self.push_transaction(transaction)
    }

    /// Returns a raw transaction (hex encoded string) from a transaction object.
    /// For unlocked basic sender accounts, the transaction will automatically be signed.
    /// Parameters:
    /// - transaction (object)
    ///
    /// The transaction looks like the following:
    /// ```text
    /// {
    ///     from: string,
    ///     fromType: number|null,
    ///     to: string|null, (null only valid for contract creation)
    ///     toType: number|null,
    ///     value: number, (in Luna)
    ///     fee: number, (in Luna)
    ///     flags: number|null,
    ///     data: string|null,
    ///     validityStartHeight: number|null,
    /// }
    /// ```
    /// Fields that can be null are optional.
    pub(crate) fn create_raw_transaction(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let mut transaction = obj_to_transaction(params.get(0).unwrap_or(&Null), self.mempool.current_height(), self.mempool.network_id())?;

        if let Some(ref unlocked_wallets) = self.unlocked_wallets {
            if transaction.sender_type == AccountType::Basic {
                if let Some(wallet_account) = unlocked_wallets.read().get(&transaction.sender) {
                    wallet_account.sign_transaction(&mut transaction);
                }
            }
        }

        let raw = Serialize::serialize_to_vec(&transaction);
        Ok(hex::encode(raw).into())
    }

    /// Creates and sends a transaction from a transaction object.
    /// Requires the sender account to be a basic account and to be unlocked.
    /// Parameters:
    /// - transaction (object)
    ///
    /// The transaction looks like the following:
    /// ```text
    /// {
    ///     from: string,
    ///     fromType: number|null,
    ///     to: string|null, (null only valid for contract creation)
    ///     toType: number|null,
    ///     value: number, (in Luna)
    ///     fee: number, (in Luna)
    ///     flags: number|null,
    ///     data: string|null,
    ///     validityStartHeight: number|null,
    /// }
    /// ```
    /// Fields that can be null are optional.
    pub(crate) fn send_transaction(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let mut transaction = obj_to_transaction(params.get(0).unwrap_or(&Null), self.mempool.current_height(), self.mempool.network_id())?;

        if let Some(ref unlocked_wallets) = self.unlocked_wallets {
            if transaction.sender_type == AccountType::Basic {
                if let Some(wallet_account) = unlocked_wallets.read().get(&transaction.sender) {
                    wallet_account.sign_transaction(&mut transaction);
                } else {
                    return Err(object! {"message" => "Sender account is locked"});
                }
            } else {
                return Err(object! {"message" => "Sender account is not a basic account"});
            }
        }

        self.push_transaction(transaction)
    }

    /// Returns the transaction for a hash if it is in the mempool and `null` otherwise.
    /// Parameters:
    /// - transactionHash (string)
    ///
    /// The transaction object returned has the following fields:
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
    ///     // Not applicable:
    ///     blockHash: null,
    ///     blockNumber: null,
    ///     timestamp: null,
    ///     confirmations: null,
    ///     transactionIndex: null,
    /// }
    /// ```
    pub(crate) fn get_transaction(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let hash = params.get(0).and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid transaction hash"})
            .and_then(|s| Blake2bHash::from_str(s)
                .map_err(|_| object! {"message" => "Invalid transaction hash"}))?;
        let transaction = self.mempool.get_transaction(&hash);
        if let Some(tx) = transaction {
            Ok(transaction_to_obj(&tx, None, None))
        } else {
            Ok(Null)
        }
    }

    // Helper functions

    pub(crate) fn push_transaction(&self, transaction: Transaction) -> Result<JsonValue, JsonValue> {
        match self.mempool.push_transaction(transaction) {
            ReturnCode::Accepted | ReturnCode::Known => Ok(object! {"message" => "Ok"}),
            code => Err(object! {"message" => format!("Rejected: {:?}", code)})
        }
    }
}

pub(crate) fn transaction_to_obj(transaction: &Transaction, context: Option<&TransactionContext>, head_height: Option<u32>) -> JsonValue {
    object! {
        "hash" => transaction.hash::<Blake2bHash>().to_hex(),
        "blockHash" => context.map(|c| c.block_hash.into()).unwrap_or(Null),
        "blockNumber" => context.map(|c| c.block_number.into()).unwrap_or(Null),
        "timestamp" => context.map(|c| (c.timestamp / 1000).into()).unwrap_or(Null),
        "timestampMillis" => context.map(|c| c.timestamp.into()).unwrap_or(Null),
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

// {
//     from: string,
//     fromType: number|null,
//     to: string,
//     toType: number|null,
//     value: number, (in Luna)
//     fee: number, (in Luna)
//     flags: number|null,
//     data: string|null,
//     validityStartHeight: number|null,
// }
pub(crate) fn obj_to_transaction(obj: &JsonValue, current_height: u32, network_id: NetworkId) -> Result<Transaction, JsonValue> {
    let from = Address::from_any_str(obj["from"].as_str()
        .ok_or_else(|| object! {"message" => "Sender address must be a string"})?)
        .map_err(|_|  object! {"message" => "Sender address invalid"})?;

    let from_type = match &obj["fromType"] {
        &JsonValue::Null => Some(AccountType::Basic),
        n @ JsonValue::Number(_) => n.as_u8().and_then(AccountType::from_int),
        _ => None
    }.ok_or_else(|| object! {"message" => "Invalid sender account type"})?;

    let to = match obj["to"] {
        JsonValue::String(ref recipient) => Some(Address::from_any_str(recipient)
            .map_err(|_|  object! {"message" => "Recipient address invalid"})?),
        _ => None,
    };

    let to_type = match &obj["toType"] {
        &JsonValue::Null => Some(AccountType::Basic),
        n @ JsonValue::Number(_) => n.as_u8().and_then(AccountType::from_int),
        _ => None
    }.ok_or_else(|| object! {"message" => "Invalid recipient account type"})?;

    let value = Coin::try_from(obj["value"].as_u64()
        .ok_or_else(|| object! {"message" => "Invalid transaction value"})?)
        .map_err(|_| object! {"message" => "Invalid transaction value"})?;

    // FIXME Wrong fee
    let fee = Coin::try_from(obj["value"].as_u64()
        .ok_or_else(|| object! {"message" => "Invalid transaction fee"})?)
        .map_err(|_| object! {"message" => "Invalid transaction fee"})?;

    let flags = obj["flags"].as_u8()
        .map_or_else(|| Some(TransactionFlags::empty()), TransactionFlags::from_bits)
        .ok_or_else(|| object! {"message" => "Invalid transaction flags"})?;

    let data = obj["data"].as_str()
        .map(hex::decode)
        .transpose().map_err(|_| object! {"message" => "Invalid transaction data"})?
        .unwrap_or_else(|| vec![]);

    let validity_start_height = match &obj["validityStartHeight"] {
        &JsonValue::Null => Some(current_height),
        n @ JsonValue::Number(_) => n.as_u32(),
        _ => None
    }.ok_or_else(|| object! {"message" => "Invalid validityStartHeight"})?;

    if to_type != AccountType::Basic
        && flags.contains(TransactionFlags::CONTRACT_CREATION)
        && to.is_none() {
        Ok(Transaction::new_contract_creation(data, from, from_type, to_type, value, fee, validity_start_height, network_id))
    } else if to.is_some()
        && !flags.contains(TransactionFlags::CONTRACT_CREATION) {
        Ok(Transaction::new_extended(from, from_type, to.unwrap(), to_type, value, fee, data, validity_start_height, network_id))
    } else {
        Err(object! {"message" => "Invalid combination of flags, toType and to."})
    }
}

pub(crate) struct TransactionContext<'a> {
    pub block_hash: &'a str,
    pub block_number: u32,
    pub index: u16,
    pub timestamp: u64, // Milliseconds
}

impl<P: ConsensusProtocol + 'static> Module for MempoolHandler<P> {
    rpc_module_methods! {
        // Transactions
        "sendRawTransaction" => send_raw_transaction,
        "createRawTransaction" => create_raw_transaction,
        "sendTransaction" => send_transaction,
        "mempoolContent" => mempool_content,
        "mempool" => mempool,
        "getTransaction" => get_transaction,
    }
}
