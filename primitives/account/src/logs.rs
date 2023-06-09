use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{
    account::{AccountType, FailReason},
    coin::Coin,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, HashAlgorithm},
    Transaction,
};

use crate::TransactionOperationReceipt;

#[derive(Debug, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
// Renaming affects only the struct names and thus their tag, the "type" field.
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum Log {
    // Used together with all transactions (inherents are excluded).
    #[serde(rename_all = "camelCase")]
    PayFee { from: Address, fee: Coin },

    // Basic account associated event.
    // Used also for every event of HTLCs, Vesting Contracts that implies a control change of the coins.
    #[serde(rename_all = "camelCase")]
    Transfer {
        from: Address,
        to: Address,
        amount: Coin,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Vec<u8>>,
    },

    #[serde(rename_all = "camelCase")]
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

    #[serde(rename_all = "camelCase")]
    HTLCTimeoutResolve { contract_address: Address },

    #[serde(rename_all = "camelCase")]
    HTLCRegularTransfer {
        contract_address: Address,
        pre_image: AnyHash,
        hash_depth: u8,
    },

    #[serde(rename_all = "camelCase")]
    HTLCEarlyResolve { contract_address: Address },

    #[serde(rename_all = "camelCase")]
    VestingCreate {
        contract_address: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        step_amount: Coin,
        total_amount: Coin,
    },

    #[serde(rename_all = "camelCase")]
    CreateValidator {
        validator_address: Address,
        reward_address: Address,
    },

    #[serde(rename_all = "camelCase")]
    UpdateValidator {
        validator_address: Address,
        old_reward_address: Address,
        new_reward_address: Option<Address>,
    },

    #[serde(rename_all = "camelCase")]
    ValidatorFeeDeduction {
        validator_address: Address,
        fee: Coin,
    },

    #[serde(rename_all = "camelCase")]
    DeactivateValidator { validator_address: Address },

    #[serde(rename_all = "camelCase")]
    ReactivateValidator { validator_address: Address },

    #[serde(rename_all = "camelCase")]
    CreateStaker {
        staker_address: Address,
        validator_address: Option<Address>,
        value: Coin,
    },

    #[serde(rename_all = "camelCase")]
    Stake {
        staker_address: Address,
        validator_address: Option<Address>,
        value: Coin,
    },

    #[serde(rename_all = "camelCase")]
    StakerFeeDeduction { staker_address: Address, fee: Coin },

    #[serde(rename_all = "camelCase")]
    UpdateStaker {
        staker_address: Address,
        old_validator_address: Option<Address>,
        new_validator_address: Option<Address>,
    },

    #[serde(rename_all = "camelCase")]
    RetireValidator { validator_address: Address },

    #[serde(rename_all = "camelCase")]
    DeleteValidator {
        validator_address: Address,
        reward_address: Address,
    },

    #[serde(rename_all = "camelCase")]
    Unstake {
        staker_address: Address,
        validator_address: Option<Address>,
        value: Coin,
    },

    #[serde(rename_all = "camelCase")]
    PayoutReward { to: Address, value: Coin },

    #[serde(rename_all = "camelCase")]
    Slash {
        validator_address: Address,
        event_block: u32,
        slot: u16,
        newly_disabled: bool,
        newly_deactivated: bool,
    },

    #[serde(rename_all = "camelCase")]
    RevertContract { contract_address: Address },

    #[serde(rename_all = "camelCase")]
    FailedTransaction {
        from: Address,
        to: Address,
        failure_reason: FailReason,
    },
}

impl Log {
    pub fn transfer_log(transaction: &Transaction) -> Self {
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

    pub fn pay_fee_log(transaction: &Transaction) -> Self {
        Log::PayFee {
            from: transaction.sender.clone(),
            fee: transaction.fee,
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
            Log::DeactivateValidator { validator_address } => validator_address == address,
            Log::ReactivateValidator { validator_address } => validator_address == address,
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
            Log::RetireValidator { validator_address } => validator_address == address,
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
            Log::Slash {
                validator_address, ..
            } => validator_address == address,
            Log::RevertContract { contract_address } => contract_address == address,
            Log::FailedTransaction { from, to, .. } => from == address || to == address,
            Log::ValidatorFeeDeduction {
                validator_address, ..
            } => validator_address == address,
            Log::StakerFeeDeduction { staker_address, .. } => staker_address == address,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionLog {
    #[serde(rename = "hash")]
    pub tx_hash: Blake2bHash,
    pub logs: Vec<Log>,
    pub failed: bool,
}

impl TransactionLog {
    pub fn new(tx_hash: Blake2bHash, logs: Vec<Log>) -> Self {
        Self {
            tx_hash,
            logs,
            failed: false,
        }
    }

    pub fn push_failed_log(&mut self, transaction: &Transaction, reason: FailReason) {
        self.failed = true;

        self.push_log(Log::FailedTransaction {
            from: transaction.sender.clone(),
            to: transaction.recipient.clone(),
            failure_reason: reason,
        })
    }

    pub fn empty() -> Self {
        Self::new(Blake2bHash::default(), vec![])
    }

    pub(crate) fn push_log(&mut self, log: Log) {
        self.logs.push(log)
    }

    #[cfg(feature = "interaction-traits")]
    pub(crate) fn clear(&mut self) {
        self.logs.clear()
    }

    pub fn rev_log(&mut self) {
        self.logs.reverse()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InherentLogger<'a> {
    inherents: Option<&'a mut Vec<Log>>,
}

impl<'a> InherentLogger<'a> {
    pub fn new(logs: &'a mut Vec<Log>) -> Self {
        InherentLogger {
            inherents: Some(logs),
        }
    }

    pub fn empty() -> Self {
        Self { inherents: None }
    }

    #[cfg(feature = "interaction-traits")]
    pub(crate) fn push_log(&mut self, log: Log) {
        if let Some(ref mut inherents) = self.inherents {
            inherents.push(log);
        }
    }

    pub fn rev_log(&mut self) {
        if let Some(ref mut inherents) = self.inherents {
            inherents.reverse()
        }
    }

    pub(crate) fn push_tx_logger(&mut self, mut tx_logger: TransactionLog) {
        if let Some(ref mut inherents) = self.inherents {
            inherents.append(&mut tx_logger.logs)
        }
    }
}

#[derive(Debug, Clone)]
pub struct BlockLogger {
    block_log: BlockLog,
}

impl BlockLogger {
    fn new(block_log: BlockLog) -> Self {
        Self { block_log }
    }

    pub fn empty() -> Self {
        Self::new(BlockLog::AppliedBlock {
            inherent_logs: vec![],
            block_hash: Default::default(),
            block_number: 0,
            timestamp: 0,
            tx_logs: vec![],
            total_tx_size: 0,
        })
    }

    pub fn empty_reverted() -> Self {
        Self::new(BlockLog::RevertedBlock {
            inherent_logs: vec![],
            block_hash: Default::default(),
            block_number: 0,
            tx_logs: vec![],
            total_tx_size: 0,
        })
    }

    pub fn new_applied(block_hash: Blake2bHash, block_number: u32, timestamp: u64) -> Self {
        Self::new(BlockLog::AppliedBlock {
            inherent_logs: vec![],
            block_hash,
            block_number,
            timestamp,
            tx_logs: vec![],
            total_tx_size: 0,
        })
    }

    pub fn new_reverted(block_hash: Blake2bHash, block_number: u32) -> Self {
        Self::new(BlockLog::RevertedBlock {
            inherent_logs: vec![],
            block_hash,
            block_number,
            tx_logs: vec![],
            total_tx_size: 0,
        })
    }

    #[cfg(feature = "interaction-traits")]
    pub(crate) fn inherent_logger(&mut self) -> InherentLogger {
        match self.block_log {
            BlockLog::RevertedBlock {
                ref mut inherent_logs,
                ..
            }
            | BlockLog::AppliedBlock {
                ref mut inherent_logs,
                ..
            } => InherentLogger {
                inherents: Some(inherent_logs),
            },
        }
    }

    #[cfg(feature = "interaction-traits")]
    pub(crate) fn new_tx_log(&mut self, tx_hash: Blake2bHash) -> &mut TransactionLog {
        let tx_log = TransactionLog::new(tx_hash, vec![]);

        match self.block_log {
            BlockLog::RevertedBlock {
                ref mut tx_logs, ..
            }
            | BlockLog::AppliedBlock {
                ref mut tx_logs, ..
            } => {
                tx_logs.push(tx_log);
                tx_logs.last_mut().unwrap()
            }
        }
    }

    pub fn build(self, size: u64) -> BlockLog {
        let mut block_log = self.block_log;
        match block_log {
            BlockLog::RevertedBlock {
                ref mut total_tx_size,
                ..
            }
            | BlockLog::AppliedBlock {
                ref mut total_tx_size,
                ..
            } => {
                *total_tx_size = size;
            }
        }
        block_log
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

    pub fn transaction_logs(&self) -> &[TransactionLog] {
        match self {
            BlockLog::AppliedBlock { ref tx_logs, .. }
            | BlockLog::RevertedBlock { ref tx_logs, .. } => tx_logs,
        }
    }

    pub fn inherent_logs(&self) -> &[Log] {
        match self {
            BlockLog::AppliedBlock {
                ref inherent_logs, ..
            }
            | BlockLog::RevertedBlock {
                ref inherent_logs, ..
            } => inherent_logs,
        }
    }
}
// This structure stores the info/data associated to a successful transaction that was committed
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

impl<T: Serialize> From<OperationInfo<T>> for AccountInfo {
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
#[derive(Default, Debug)]
pub struct BatchInfo {
    pub receipts: Vec<TransactionOperationReceipt>,
    pub tx_logs: Vec<TransactionLog>,
    pub inherent_logs: Vec<Log>,
}

impl BatchInfo {
    pub fn new(
        receipts: Vec<TransactionOperationReceipt>,
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
pub struct OperationInfo<T: Serialize> {
    pub receipt: Option<T>,
    pub logs: Vec<Log>,
    pub missing_account: bool,
}

impl<T: Serialize> OperationInfo<T> {
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

impl<T: Serialize> MissingInfo for OperationInfo<T> {
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
