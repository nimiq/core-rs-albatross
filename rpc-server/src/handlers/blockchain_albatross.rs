use std::borrow::Borrow;
use std::convert::TryInto;
use std::sync::Arc;

use json::{object, JsonValue, Null};

use account::staking_contract::{InactiveStake, InactiveValidator, Validator};
use account::Account;
use block_albatross::{signed, Block, ForkProof};
use blockchain_albatross::Blockchain;
use blockchain_base::AbstractBlockchain;
use bls::CompressedPublicKey as BlsPublicKey;
use hash::{Blake2bHash, Hash};
use keys::Address;
use network_primitives::networks::NetworkInfo;
use primitives::policy;
use primitives::slot::{Slot, SlotBand, Slots};

use crate::handler::Method;
use crate::handlers::blockchain::BlockchainHandler;
use crate::handlers::mempool::{transaction_to_obj, TransactionContext};
use crate::handlers::Module;
use crate::rpc_not_implemented;
use json::object::Object;

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
        Ok(self.block_to_obj(
            &block,
            params.get(1).and_then(|v| v.as_bool()).unwrap_or(false),
        ))
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
        let block = self
            .generic
            .block_by_number(params.get(0).unwrap_or(&Null))?;
        Ok(self.block_to_obj(
            &block,
            params.get(1).and_then(|v| v.as_bool()).unwrap_or(false),
        ))
    }

    /// Returns the producer of a block given the block and view number.
    /// The block number has to be less or equal to the current chain height
    /// and greater than that of the second last known macro block.
    /// Parameters:
    /// - block_number (number)
    /// - view_number (number) (optional)
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
    pub(crate) fn get_slot_at(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        // Parse block number argument
        let block_number = params
            .get(0)
            .ok_or_else(|| object! {"message" => "First argument must be block number"})
            .and_then(|n| self.generic.parse_block_number(n))?;

        // Check if it's not a macro block
        //
        // TODO: Macro blocks have a slot too. It's just only for the proposal.
        if policy::is_macro_block_at(block_number) {
            // TODO: Macro blocks have a proposer
            return Err(object! {"message" => "Block is a macro block"});
        }

        // Parse view number or use the view number which eventually produced the block
        let view_number = params
            .get(1)
            .map(|param| {
                param
                    .as_u32()
                    .ok_or_else(|| object! {"message" => "Invalid view number"})
            })
            .transpose()?;

        let view_number = if let Some(view_number) = view_number {
            view_number
        } else {
            let block = self
                .blockchain
                .get_block_at(block_number, true)
                .ok_or(object! {"message" => "Unknown block"})?;
            block.view_number()
        };

        // get slot and slot number
        let (slot, slot_number) = self
            .blockchain
            .get_slot_at(block_number, view_number, None)
            .ok_or(object! {"message" => "Block number out of range"})?;

        Ok(Self::slot_to_obj(&slot, slot_number))
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

        let current_slashed_set = JsonValue::Array(
            state
                .current_slashed_set()
                .iter()
                .map(|slot_number| JsonValue::Number(slot_number.into()))
                .collect(),
        );

        let last_slashed_set = JsonValue::Array(
            state
                .last_slashed_set()
                .iter()
                .map(|slot_number| JsonValue::Number(slot_number.into()))
                .collect(),
        );

        Ok(object! {
            "blockNumber" => state.block_number(),
            "currentSlashedSlotNumbers" => current_slashed_set,
            "lastSlashedSlotNumbers" => last_slashed_set,
        })
    }

    // Transactions

    /// Retrieves information about a transaction from its hex encoded form.
    /// Parameters:
    /// - transaction (string): Hex encoded transaction.
    ///
    /// Returns an info object:
    pub(crate) fn get_raw_transaction_info(
        &self,
        _params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
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
    pub(crate) fn get_transaction_by_hash(
        &self,
        _params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
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
    pub(crate) fn get_transaction_receipt(
        &self,
        _params: &[JsonValue],
    ) -> Result<JsonValue, JsonValue> {
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

    // Accounts

    // Lists all stakes
    pub(crate) fn list_stakes(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let genesis_account = NetworkInfo::from_network_id(self.blockchain.network_id)
            .validator_registry_address()
            .unwrap();
        let account = self.blockchain.get_account(&genesis_account);
        let contract = match account {
            Account::Staking(c) => c,
            _ => return Err("No contract at staking contract address".into()),
        };
        let active_validators: Vec<JsonValue> = contract
            .active_validators_by_key
            .values()
            .map(|validator| {
                BlockchainAlbatrossHandler::active_validator_to_obj(validator.borrow())
            })
            .collect();
        let inactive_validators: Vec<JsonValue> = contract
            .inactive_validators_by_key
            .iter()
            .map(|(key, validator)| {
                BlockchainAlbatrossHandler::inactive_validator_to_obj(key, validator)
            })
            .collect();
        let inactive_stakes: Vec<JsonValue> = contract
            .inactive_stake_by_address
            .iter()
            .map(|(address, stake)| {
                BlockchainAlbatrossHandler::inactive_stake_to_obj(address, stake)
            })
            .collect();
        Ok(object! {
            "activeValidators" => active_validators,
            "inactiveValidators" => inactive_validators,
            "inactiveStakes" => inactive_stakes,
        })
    }

    // Helper functions

    fn proof_to_object<M: signed::Message>(proof: &signed::AggregateProof<M>) -> JsonValue {
        object! {
            "signature" => format!("{}", proof.signature),
            "signers" => proof.signers.iter().collect::<Vec<usize>>(),
        }
    }

    fn block_to_obj(&self, block: &Block, include_transactions: bool) -> JsonValue {
        let hash = block.hash().to_hex();
        let blockchain_height = self.blockchain.height();

        match block {
            Block::Macro(ref block) => {
                let slots = block.clone().try_into().ok();

                let justification = block
                    .justification
                    .as_ref()
                    .map(|pbft_proof| {
                        let validators = block.header.validators.clone().into();
                        object! {
                            "votes" => pbft_proof.votes(&validators)
                                .map(JsonValue::from).unwrap_or(JsonValue::Null),
                            "prepare" => Self::proof_to_object(&pbft_proof.prepare),
                            "commit" => Self::proof_to_object(&pbft_proof.commit),
                        }
                    })
                    .unwrap_or(JsonValue::Null);

                let slashed_slots = block
                    .extrinsics
                    .as_ref()
                    .map(|extrinsics| {
                        JsonValue::Array(
                            extrinsics
                                .slashed_set
                                .iter()
                                .map(|slot_number| JsonValue::Number(slot_number.into()))
                                .collect(),
                        )
                    })
                    .unwrap_or(JsonValue::Null);

                object! {
                    "type" => "macro",
                    "hash" => hash.clone(),
                    "blockNumber" => block.header.block_number,
                    "viewNumber" => block.header.view_number,
                    "epoch" => policy::epoch_at(block.header.block_number),
                    "parentMacroHash" => block.header.parent_macro_hash.to_hex(),
                    "parentHash" => block.header.parent_hash.to_hex(),
                    "seed" => block.header.seed.to_string(),
                    "stateRoot" => block.header.state_root.to_hex(),
                    "extrinsicsRoot" => block.header.extrinsics_root.to_hex(),
                    "timestamp" => block.header.timestamp,
                    "slots" => slots.as_ref().map(Self::slots_to_obj).unwrap_or(Null),
                    "slashedSlots" => slashed_slots,
                    "justification" => justification,
                }
            }
            Block::Micro(ref block) => {
                let producer = self
                    .blockchain
                    .get_slot_at(block.header.block_number, block.header.view_number, None)
                    .map(|(slot, slot_number)| Self::slot_to_obj(&slot, slot_number))
                    .unwrap_or(JsonValue::Null);

                object! {
                    "type" => "micro",
                    "hash" => hash.clone(),
                    "blockNumber" => block.header.block_number,
                    "viewNumber" => block.header.view_number,
                    "epoch" => policy::epoch_at(block.header.block_number),
                    "parentHash" => block.header.parent_hash.to_hex(),
                    "stateRoot" => block.header.state_root.to_hex(),
                    "extrinsicsRoot" => block.header.extrinsics_root.to_hex(),
                    "seed" => block.header.seed.to_string(),
                    "timestamp" => block.header.timestamp,
                    "extraData" => block.extrinsics.as_ref().map(|body| hex::encode(&body.extra_data).into()).unwrap_or(Null),
                    "forkProofs" => block.extrinsics.as_ref().map(|body| JsonValue::Array(body.fork_proofs.iter().map(Self::fork_proof_to_obj).collect())).unwrap_or(Null),
                    "transactions" => JsonValue::Array(block.extrinsics.as_ref().map(|body| if include_transactions {
                        body.transactions.iter().enumerate().map(|(i, tx)| transaction_to_obj(tx, Some(&TransactionContext {
                            block_hash: &hash,
                            block_number: block.header.block_number,
                            index: i as u16,
                            timestamp: block.header.timestamp,
                        }), Some(blockchain_height ))).collect()
                    } else {
                        body.transactions.iter().map(|tx| tx.hash::<Blake2bHash>().to_hex().into()).collect()
                    }).unwrap_or_else(Vec::new)),
                    "producer" => producer,
                    "justification" => object! {
                        "signature" => block.justification.signature.to_hex(),
                        "view_change_proof" => block.justification.view_change_proof.as_ref()
                            .map(Self::proof_to_object)
                            .unwrap_or(JsonValue::Null),
                    },
                }
            }
        }
    }

    fn slots_to_obj(slots: &Slots) -> JsonValue {
        JsonValue::Array(
            slots
                .combined()
                .into_iter()
                .map(|(slot, first_slot_number)| {
                    object! {
                        "firstSlotNumber" => first_slot_number,
                        "numSlots" => slot.validator_slot.num_slots(),
                        "publicKey" => slot.public_key().to_string(),
                        "rewardAddress" => slot.reward_address().to_user_friendly_address(),
                    }
                })
                .collect(),
        )
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

    fn slot_to_obj(slot: &Slot, slot_number: u16) -> JsonValue {
        object! {
            "index" => slot_number,
            "publicKey" => slot.public_key().to_string(),
            "rewardAddress" => slot.reward_address().to_user_friendly_address(),
        }
    }

    fn active_validator_to_obj(validator: &Arc<Validator>) -> JsonValue {
        let mut stakes = Object::new();

        for (address, &stake) in validator.active_stake_by_address.read().iter() {
            stakes.insert(&address.to_user_friendly_address(), u64::from(stake).into());
        }

        object! {
            "publicKey" => hex::encode(&validator.validator_key),
            "balance" => u64::from(validator.balance),
            "rewardAddress" => validator.reward_address.to_user_friendly_address(),
            "stakes" => JsonValue::Object(stakes),
        }
    }

    fn inactive_validator_to_obj(_key: &BlsPublicKey, validator: &InactiveValidator) -> JsonValue {
        object! {
            "validator" => Self::active_validator_to_obj(&validator.validator),
            "retireTime" => validator.retire_time,
        }
    }

    fn inactive_stake_to_obj(address: &Address, stake: &InactiveStake) -> JsonValue {
        object! {
            "stakerAddress" => address.to_user_friendly_address(),
            "balance" => u64::from(stake.balance),
            "retireTime" => stake.retire_time,
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
        "get_slot_at" => get_slot_at,
        "getBlockTransactionCountByHash" => generic.get_block_transaction_count_by_hash,
        "getBlockTransactionCountByNumber" => generic.get_block_transaction_count_by_number,
        "slotState" => slot_state,

        // Accounts
        "getBalance" => generic.get_balance,
        "listStakes" => list_stakes,
    }
}
