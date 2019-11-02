use std::convert::TryInto;
use std::iter::FromIterator;
use std::sync::Arc;

use json::{JsonValue, Null};

use block_albatross::{Block, ForkProof};
use blockchain_albatross::Blockchain;
use blockchain_albatross::reward_registry::SlashedSlots;
use hash::{Blake2bHash, Hash};
use primitives::policy;
use primitives::validators::{IndexedSlot, Slots};

use crate::handler::Method;
use crate::handlers::Module;
use crate::handlers::blockchain::BlockchainHandler;
use crate::handlers::mempool::{transaction_to_obj, TransactionContext};
use crate::rpc_not_implemented;

pub struct BlockchainAlbatrossHandler {
    pub blockchain: Arc<Blockchain>,
    generic: BlockchainHandler<Blockchain>,
}

impl BlockchainAlbatrossHandler {
    pub fn new(blockchain: Arc<Blockchain>) -> Self {
        BlockchainAlbatrossHandler {
            generic: BlockchainHandler::new(blockchain.clone()),
            blockchain,
        }
    }

    // Blocks

    /// Returns the current epoch number.
    pub(crate) fn epoch_number(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        Ok(policy::epoch_at(self.blockchain.height()).into())
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
    ///     timestampMillis: number,
    ///     transactions: Array<transaction_objects> | Array<string>, (depends on includeTransactions),
    /// }
    /// ```
    pub(crate) fn get_block_by_hash(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
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
    ///     timestampMillis: number,
    ///     transactions: Array<transaction_objects> | Array<string>, (depends on includeTransactions),
    /// }
    /// ```
    pub(crate) fn get_block_by_number(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let block = self.generic.block_by_number(params.get(0).unwrap_or(&Null))?;
        Ok(self.block_to_obj(&block, params.get(1).and_then(|v| v.as_bool()).unwrap_or(false)))
    }

    /// Returns the producer of a block given the block and view number.
    /// The block number has to be less or equal to the current chain height
    /// and greater than that of the second last known macro block.
    /// Parameters:
    /// - block_number (number)
    /// TODO - view_number (number) (optional)
    ///
    /// The producer object contains:
    /// ```text
    /// {
    ///     index: number,
    ///     publicKey: string,
    ///     stakerAddress: string,
    ///     rewardAddress: string,
    /// }
    /// ```
    pub(crate) fn get_producer(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let block_number = params.get(0)
            .ok_or(object!{"message" => "First argument must be block number"})
            .and_then(|n| self.generic.parse_block_number(n))?;

        if policy::is_macro_block_at(block_number) {
            return Err(object!{"message" => "Block is a macro block"});
        }

        let block = self.blockchain.get_block_at(block_number, true)
            .ok_or(object!{"message" => "Unknown block"})?;
        let producer = self.blockchain.get_block_producer_at(block_number, block.view_number(), None)
            .ok_or(object!{"message" => "Block number out of range"})?;

        Ok(Self::indexed_slot_to_obj(&producer))
    }

    /// Returns the state of the slots.
    ///
    /// The state object contains:
    /// ```text
    /// {
    ///     blockNumber: number,
    ///     currentSlots: SlotList,
    ///     lastSlots: SlotList
    /// }
    /// ```
    ///
    /// A SlotList is an array with objects looking like:
    /// ```text
    /// {
    ///     index: number,
    ///     publicKey: string,
    ///     stakerAddress: string,
    ///     rewardAddress: string,
    /// }
    /// ```
    pub(crate) fn slot_state(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let state = self.blockchain.state();

        let current_slots = state.current_slots().ok_or(object!{"message" => "No current slots"})?;
        let current_slashed_set = state.current_slashed_set();
        let current_slashed_slots = SlashedSlots::new(&current_slots, &current_slashed_set);

        let last_slots = state.last_slots().ok_or(object!{"message" => "No last slots"})?;
        let last_slashed_set = state.last_slashed_set();
        let last_slashed_slots = SlashedSlots::new(&last_slots, &last_slashed_set);

        Ok(object!{
            "blockNumber" => state.block_number(),
            "currentSlots" => Self::slashed_slots_to_obj(&current_slashed_slots),
            "lastSlots" => Self::slashed_slots_to_obj(&last_slashed_slots),
        })
    }

    // Transactions

    /// Retrieves information about a transaction from its hex encoded form.
    /// Parameters:
    /// - transaction (string): Hex encoded transaction.
    ///
    /// Returns an info object:
    pub(crate) fn get_raw_transaction_info(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        /*
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
        */

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
    ///     timestampMillis: number,
    ///     confirmations: number,
    ///     transactionIndex: number,
    /// }
    /// ```
    pub(crate) fn get_transaction_by_hash(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        rpc_not_implemented()
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
    ///     timestampMillis: number,
    ///     confirmations: number,
    ///     transactionIndex: number,
    /// }
    /// ```
    pub(crate) fn get_transaction_receipt(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        /*
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
        */
        rpc_not_implemented()
    }

    // Helper functions

    fn block_to_obj(&self, block: &Block, include_transactions: bool) -> JsonValue {
        let hash = block.hash().to_hex();
        let height = self.blockchain.height();
        match block {
            Block::Macro(ref block) => object! {
                "type" => "macro",
                "hash" => hash.clone(),
                "blockNumber" => block.header.block_number,
                "viewNumber" => block.header.view_number,
                "parentMacroHash" => block.header.parent_macro_hash.to_hex(),
                "parentHash" => block.header.parent_hash.to_hex(),
                "seed" => hex::encode(&block.header.seed),
                "stateRoot" => block.header.state_root.to_hex(),
                "extrinsicsRoot" => block.header.extrinsics_root.to_hex(),
                "timestamp" => block.header.timestamp / 1000,
                "timestampMillis" => block.header.timestamp,
                "slots" => block.clone().try_into().as_ref().map(Self::slots_to_obj).unwrap_or(Null),
                "slashFine" => block.extrinsics.as_ref().map(|body| JsonValue::from(u64::from(body.slash_fine))).unwrap_or(Null),
            },
            Block::Micro(ref block) => object! {
                "type" => "micro",
                "hash" => hash.clone(),
                "blockNumber" => block.header.block_number,
                "viewNumber" => block.header.view_number,
                "parentHash" => block.header.parent_hash.to_hex(),
                "stateRoot" => block.header.state_root.to_hex(),
                "extrinsicsRoot" => block.header.extrinsics_root.to_hex(),
                "seed" => hex::encode(&block.header.seed),
                "timestamp" => block.header.timestamp,
                "extraData" => block.extrinsics.as_ref().map(|body| hex::encode(&body.extra_data).into()).unwrap_or(Null),
                "forkProofs" => block.extrinsics.as_ref().map(|body| JsonValue::Array(body.fork_proofs.iter().map(Self::fork_proof_to_obj).collect())).unwrap_or(Null),
                "transactions" => JsonValue::Array(block.extrinsics.as_ref().map(|body| if include_transactions {
                    body.transactions.iter().enumerate().map(|(i, tx)| transaction_to_obj(tx, Some(&TransactionContext {
                        block_hash: &hash,
                        block_number: block.header.block_number,
                        index: i as u16,
                        timestamp: block.header.timestamp,
                    }), Some(height))).collect()
                } else {
                    body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
                }).unwrap_or_else(Vec::new)),
                "signature" => hex::encode(&block.justification.signature),
            }
        }
    }

    fn slots_to_obj(slots: &Slots) -> JsonValue {
        JsonValue::Array(Vec::from_iter(slots.iter()
            .enumerate()
            .map(|(i, slot)| object! {
                "index" => i,
                "publicKey" => hex::encode(&slot.public_key),
                "stakerAddress" => slot.staker_address.to_hex(),
                "rewardAddress" => slot.reward_address().to_hex(),
            })
        ))
    }

    fn slashed_slots_to_obj(slashed_slots: &SlashedSlots) -> JsonValue {
        JsonValue::Array(Vec::from_iter(slashed_slots.slot_states()
            .enumerate()
            .map(|(i, (slot, enabled))| object! {
                "index" => i,
                "publicKey" => hex::encode(&slot.public_key),
                "stakerAddress" => slot.staker_address.to_hex(),
                "rewardAddress" => slot.reward_address().to_hex(),
                "slashed" => !enabled,
            })
        ))
    }

    fn fork_proof_to_obj(fork_proof: &ForkProof) -> JsonValue {
        object! {
            "blockNumber" => fork_proof.header1.block_number,
            "viewNumber" => fork_proof.header1.view_number,
            "parentHash" => fork_proof.header1.parent_hash.to_hex(),
            "hashes" => vec![
                fork_proof.header1.hash::<Blake2bHash>().to_hex(),
                fork_proof.header2.hash::<Blake2bHash>().to_hex(),
            ]
        }
    }

    fn indexed_slot_to_obj(idx_slot: &IndexedSlot) -> JsonValue {
        object! {
            "index" => idx_slot.idx,
            "publicKey" => hex::encode(&idx_slot.slot.public_key),
            "stakerAddress" => idx_slot.slot.staker_address.to_hex(),
            "rewardAddress" => idx_slot.slot.reward_address().to_hex(),
        }
    }
}

impl Module for BlockchainAlbatrossHandler {
    rpc_module_methods! {
        // Transactions
        "getRawTransactionInfo" => get_raw_transaction_info,
        "getTransactionByHash" => get_transaction_by_hash,
        "getTransactionReceipt" => get_transaction_receipt,
        "getTransactionByBlockHashAndIndex" => generic.get_transaction_by_block_hash_and_index,
        "getTransactionByBlockNumberAndIndex" => generic.get_transaction_by_block_number_and_index,
        "getTransactionsByAddress" => generic.get_transactions_by_address,

        // Blockchain
        "blockNumber" => generic.block_number,
        "epochNumber" => epoch_number,
        "getBlockByHash" => get_block_by_hash,
        "getBlockByNumber" => get_block_by_number,
        "getProducer" => get_producer,
        "getBlockTransactionCountByHash" => generic.get_block_transaction_count_by_hash,
        "getBlockTransactionCountByNumber" => generic.get_block_transaction_count_by_number,
        "slotState" => slot_state,

        // Accounts
        "getBalance" => generic.get_balance,
    }
}
