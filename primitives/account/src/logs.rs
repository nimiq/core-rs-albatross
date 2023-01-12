use crate::Receipt;
use beserial::Serialize as BeSerialize;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, HashAlgorithm},
    Transaction,
};

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
        #[cfg_attr(
            feature = "serde-derive",
            serde(skip_serializing_if = "Option::is_none")
        )]
        data: Option<Vec<u8>>,
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

    #[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
    FailedTransaction {
        from: Address,
        to: Address,
        failure_reason: String,
    },
}

impl Log {
    pub fn transfer_log_from_transaction(transaction: &Transaction) -> Self {
        Log::Transfer {
            from: transaction.sender.clone(),
            to: transaction.recipient.clone(),
            amount: transaction.value,
            data: if !transaction.data.is_empty()
                && transaction.recipient_type == AccountType::Basic
            {
                Some(transaction.data.clone())
            } else {
                None
            },
        }
    }

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
            Log::FailedTransaction { from, to, .. } => from == address || to == address,
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
pub enum BlockLog {
    AppliedBlock {
        inherent_logs: Vec<Log>,
        block_hash: Blake2bHash,
        block_number: u32,
        timestamp: u64,
        tx_logs: Vec<TransactionLog>,
        total_tx_size: u64,
    },

    RevertedBlock {
        inherent_logs: Vec<Log>,
        block_hash: Blake2bHash,
        block_number: u32,
        tx_logs: Vec<TransactionLog>,
        total_tx_size: u64,
    },
}

impl BlockLog {
    pub fn is_revert_block_log(&self) -> bool {
        match self {
            BlockLog::AppliedBlock { .. } => false,
            BlockLog::RevertedBlock { .. } => true,
        }
    }

    pub fn total_tx_size(&self) -> u64 {
        match self {
            BlockLog::AppliedBlock { total_tx_size, .. } => *total_tx_size,
            BlockLog::RevertedBlock { total_tx_size, .. } => *total_tx_size,
        }
    }
}
// This structure stores the info/data associated to a sucessful transaction that was committed
pub struct TransactionInfo {
    pub sender_info: Option<AccountInfo>,
    pub recipient_info: Option<AccountInfo>,
    pub create_info: Option<AccountInfo>,
}

impl TransactionInfo {
    pub fn new() -> Self {
        Self {
            sender_info: None,
            recipient_info: None,
            create_info: None,
        }
    }
}
impl Default for TransactionInfo {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RevertTransactionLogs {
    pub contract_log: Vec<Log>,
    pub recipient_log: Vec<Log>,
    pub sender_log: Vec<Log>,
}

impl RevertTransactionLogs {
    pub fn new() -> Self {
        Self {
            contract_log: Vec::new(),
            recipient_log: Vec::new(),
            sender_log: Vec::new(),
        }
    }
}

impl Default for RevertTransactionLogs {
    fn default() -> Self {
        Self::new()
    }
}

/// Account Info is used as a return type of transactions to the blockchain and stores the logs and receipt associated with the transaction
#[derive(Debug, PartialEq, Eq)]
pub struct AccountInfo {
    pub receipt: Option<Vec<u8>>,
    pub logs: Vec<Log>,
    /// true if the account is in the incomplete part of the trie, false otherwise
    pub missing_account: bool,
}

impl AccountInfo {
    pub fn new(receipt: Option<Vec<u8>>, logs: Vec<Log>, missing_account: bool) -> Self {
        Self {
            receipt,
            logs,
            missing_account,
        }
    }

    pub fn with_receipt(receipt: Option<Vec<u8>>, logs: Vec<Log>) -> Self {
        Self {
            receipt,
            logs,
            missing_account: false,
        }
    }
}

impl<T: BeSerialize> From<OperationInfo<T>> for AccountInfo {
    fn from(op_info: OperationInfo<T>) -> Self {
        AccountInfo::new(
            op_info.receipt.map(|receipt| receipt.serialize_to_vec()),
            op_info.logs,
            op_info.missing_account,
        )
    }
}

// Batch Info is used as a return type of multiple transactions or batches applied to the blockchain.
// It stores the transaction logs, inherent logs and receipts associated with the multiple transactions of the batch.
// Along with the result of applying each individual transaction
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

/// Operation Info is used as a return type of some staker and validator related transactions.
/// It stores the transaction logs generated by the transaction and the receipts.
/// The receipts can either be represented in bytes or in any receipt type defined.
/// It also stores whether the account is in the incomplete part of the trie.
#[derive(Debug, PartialEq, Eq)]
pub struct OperationInfo<T: BeSerialize> {
    pub receipt: Option<T>,
    pub logs: Vec<Log>,
    pub missing_account: bool,
}

impl<T: BeSerialize> OperationInfo<T> {
    pub fn new(receipt: Option<T>, logs: Vec<Log>, missing_account: bool) -> Self {
        Self {
            receipt,
            logs,
            missing_account,
        }
    }

    pub fn with_receipt(receipt: T, logs: Vec<Log>) -> Self {
        Self {
            receipt: Some(receipt),
            logs,
            missing_account: false,
        }
    }
}

pub trait MissingInfo {
    fn missing() -> Self;
}

impl<T: BeSerialize> MissingInfo for OperationInfo<T> {
    fn missing() -> Self {
        Self {
            receipt: None,
            logs: vec![],
            missing_account: true,
        }
    }
}

impl MissingInfo for AccountInfo {
    fn missing() -> Self {
        Self {
            receipt: None,
            logs: vec![],
            missing_account: true,
        }
    }
}

impl MissingInfo for Vec<Log> {
    fn missing() -> Self {
        vec![]
    }
}
