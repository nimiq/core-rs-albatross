use crate::Receipt;
use beserial::Serialize as BeSerialize;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{AnyHash, HashAlgorithm};

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
// Renaming affects only the struct names and thus their tag, the "type" field.
#[cfg_attr(
    feature = "serde-derive",
    serde(rename_all = "kebab-case", tag = "type")
)]
pub enum Log {
    // Used together with all transactions (inherents are excluded).
    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    PayFee { from: Address, fee: Coin },

    // Basic account associated event.
    // Used also for every event of HTLCs, Vesting Contracts that implies a control change of the coins.
    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    Transfer {
        from: Address,
        to: Address,
        amount: Coin,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    HTLCCreate {
        contract_address: Address,
        sender: Address,
        recipient: Address,
        hash_algorithm: HashAlgorithm,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        total_amount: Coin,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    HTLCTimeoutResolve { contract_address: Address },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    HTLCRegularTransfer {
        contract_address: Address,
        pre_image: AnyHash,
        hash_depth: u8,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    HTLCEarlyResolve { contract_address: Address },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    VestingCreate {
        contract_address: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        step_amount: Coin,
        total_amount: Coin,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    CreateValidator {
        validator_address: Address,
        reward_address: Address,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    UpdateValidator {
        validator_address: Address,
        old_reward_address: Address,
        new_reward_address: Option<Address>,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    InactivateValidator { validator_address: Address },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    ReactivateValidator { validator_address: Address },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    UnparkValidator { validator_address: Address },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    CreateStaker {
        staker_address: Address,
        validator_address: Option<Address>,
        value: Coin,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    Stake {
        staker_address: Address,
        validator_address: Option<Address>,
        value: Coin,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    UpdateStaker {
        staker_address: Address,
        old_validator_address: Option<Address>,
        new_validator_address: Option<Address>,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    DeleteValidator {
        validator_address: Address,
        reward_address: Address,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    Unstake {
        staker_address: Address,
        validator_address: Option<Address>,
        value: Coin,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    PayoutReward { to: Address, value: Coin },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    Park {
        validator_address: Address,
        event_block: u32,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    Slash {
        validator_address: Address,
        event_block: u32,
        slot: u16,
        newly_disabled: bool,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    RevertContract { contract_address: Address },
}

impl Log {
    pub fn is_related_to_address(&self, address: &Address) -> bool {
        match self {
            Log::PayFee { from, .. } => from == address,
            Log::Transfer { from, to, .. } => from == address || to == address,
            Log::HTLCCreate {
                contract_address,
                sender,
                recipient,
                ..
            } => contract_address == address || sender == address || recipient == address,
            Log::HTLCTimeoutResolve { contract_address } => contract_address == address,
            Log::HTLCRegularTransfer {
                contract_address, ..
            } => contract_address == address,
            Log::HTLCEarlyResolve { contract_address } => contract_address == address,
            Log::VestingCreate {
                contract_address,
                owner,
                ..
            } => contract_address == address || owner == address,
            Log::CreateValidator {
                validator_address,
                reward_address,
            } => validator_address == address || reward_address == address,
            Log::UpdateValidator {
                validator_address,
                old_reward_address,
                new_reward_address,
            } => {
                validator_address == address
                    || old_reward_address == address
                    || new_reward_address
                        .as_ref()
                        .map(|new_reward_address| new_reward_address == address)
                        .unwrap_or(false)
            }
            Log::InactivateValidator { validator_address } => validator_address == address,
            Log::ReactivateValidator { validator_address } => validator_address == address,
            Log::UnparkValidator { validator_address } => validator_address == address,
            Log::CreateStaker {
                staker_address,
                validator_address,
                ..
            } => {
                staker_address == address
                    || validator_address
                        .as_ref()
                        .map(|validator_address| validator_address == address)
                        .unwrap_or(false)
            }
            Log::Stake {
                staker_address,
                validator_address,
                ..
            } => {
                staker_address == address
                    || validator_address
                        .as_ref()
                        .map(|validator_address| validator_address == address)
                        .unwrap_or(false)
            }
            Log::UpdateStaker {
                staker_address,
                old_validator_address,
                new_validator_address,
            } => {
                staker_address == address
                    || old_validator_address
                        .as_ref()
                        .map(|old_address| old_address == address)
                        .unwrap_or(false)
                    || new_validator_address
                        .as_ref()
                        .map(|new_address| new_address == address)
                        .unwrap_or(false)
            }
            Log::DeleteValidator {
                validator_address,
                reward_address,
            } => validator_address == address || reward_address == address,
            Log::Unstake {
                staker_address,
                validator_address,
                ..
            } => {
                staker_address == address
                    || validator_address
                        .as_ref()
                        .map(|validator_address| validator_address == address)
                        .unwrap_or(false)
            }
            Log::PayoutReward { to, .. } => to == address,
            Log::Park {
                validator_address, ..
            } => validator_address == address,
            Log::Slash {
                validator_address, ..
            } => validator_address == address,
            Log::RevertContract { contract_address } => contract_address == address,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
pub struct TransactionLog {
    #[cfg_attr(feature = "serde-derive", serde(rename = "hash"))]
    pub tx_hash: Blake2bHash,
    pub logs: Vec<Log>,
}

impl TransactionLog {
    pub fn new(tx_hash: Blake2bHash, logs: Vec<Log>) -> Self {
        Self { tx_hash, logs }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
// Renaming affects only the struct names and thus their tag, the "type" field.
#[cfg_attr(
    feature = "serde-derive",
    serde(rename_all = "kebab-case", tag = "type")
)]
pub enum BlockLog {
    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    AppliedBlock {
        #[cfg_attr(feature = "serde-derive", serde(rename = "inherents"))]
        inherent_logs: Vec<Log>,
        block_hash: Blake2bHash,
        block_number: u32,
        timestamp: u64,
        #[cfg_attr(feature = "serde-derive", serde(rename = "transactions"))]
        tx_logs: Vec<TransactionLog>,
    },

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    RevertedBlock {
        #[cfg_attr(feature = "serde-derive", serde(rename = "inherents"))]
        inherent_logs: Vec<Log>,
        block_hash: Blake2bHash,
        block_number: u32,
        #[cfg_attr(feature = "serde-derive", serde(rename = "transactions"))]
        tx_logs: Vec<TransactionLog>,
    },
}

impl BlockLog {
    pub fn is_revert_block_log(&self) -> bool {
        match self {
            BlockLog::AppliedBlock { .. } => false,
            BlockLog::RevertedBlock { .. } => true,
        }
    }
}

// Account Info is used as a return type of transactions to the blockchain and stores the logs and receipt associated with the transaction
#[derive(Debug, PartialEq, Eq)]
pub struct AccountInfo {
    pub receipt: Option<Vec<u8>>,
    pub logs: Vec<Log>,
}

impl AccountInfo {
    pub fn new(receipt: Option<Vec<u8>>, logs: Vec<Log>) -> Self {
        Self { receipt, logs }
    }

    pub fn with_option_receipt<T: BeSerialize>(op_info: OperationInfo<Option<T>>) -> Self {
        let serialized_receipt = op_info.receipt.map(|receipt| receipt.serialize_to_vec());
        AccountInfo::new(serialized_receipt, op_info.logs)
    }
}

impl<T: BeSerialize> From<OperationInfo<T>> for AccountInfo {
    fn from(op_info: OperationInfo<T>) -> Self {
        AccountInfo::new(
            op_info.receipt.map(|receipt| receipt.serialize_to_vec()),
            op_info.logs,
        )
    }
}

// Batch Info is used as a return type of multiple transactions or batches applied to the blockchain.
// It stores the transaction logs, inherent logs and receipts associated with the multiple transactions of the batch.
#[derive(Default, Eq, PartialEq, Debug)]
pub struct BatchInfo {
    pub receipts: Vec<Receipt>,
    pub tx_logs: Vec<TransactionLog>,
    pub inherent_logs: Vec<Log>,
}

impl BatchInfo {
    pub fn new(
        receipts: Vec<Receipt>,
        tx_logs: Vec<TransactionLog>,
        inherent_logs: Vec<Log>,
    ) -> Self {
        Self {
            receipts,
            tx_logs,
            inherent_logs,
        }
    }
}

// Operation Info is used as a return type of some staker and validator related transactions.
// It stores the transaction logs generated by the transaction and the receipts.
// The receipts can either be represented in bytes or in any receipt type defined.
#[derive(Debug, PartialEq, Eq)]
pub struct OperationInfo<T: BeSerialize> {
    pub receipt: Option<T>,
    pub logs: Vec<Log>,
}

impl<T: BeSerialize> OperationInfo<T> {
    pub fn new(receipt: Option<T>, logs: Vec<Log>) -> Self {
        Self { receipt, logs }
    }

    pub fn with_receipt(receipt: T, logs: Vec<Log>) -> Self {
        Self {
            receipt: Some(receipt),
            logs,
        }
    }
}
