use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountType;
use nimiq_primitives::{account::AccountError, coin::Coin};
use nimiq_serde::{Deserialize, Serialize};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{
    account::vesting_contract::CreationTransactionData, inherent::Inherent, SignatureProof,
    Transaction,
};

use crate::{convert_receipt, AccountReceipt};
#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
    },
    reserved_balance::ReservedBalance,
    Account, BlockState, InherentLogger, Log, TransactionLog,
};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct VestingContract {
    /// Total balance of the contract.
    pub balance: Coin,
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,
    /// The block height at which the release schedule starts.
    #[serde(with = "nimiq_serde::fixint::be")]
    pub start_time: u64,
    /// The frequency at which funds are released.
    #[serde(with = "nimiq_serde::fixint::be")]
    pub time_step: u64,
    /// The amount released at each time_step.
    pub step_amount: Coin,
    /// Initially locked balance.
    pub total_amount: Coin,
}

#[cfg(feature = "interaction-traits")]
impl VestingContract {
    fn can_change_balance(
        &self,
        transaction: &Transaction,
        new_balance: Coin,
        block_state: &BlockState,
    ) -> Result<(), AccountError> {
        // Check vesting min cap.
        let min_cap = self.min_cap(block_state.time);

        if new_balance < min_cap {
            return Err(AccountError::InsufficientFunds {
                balance: self.balance - min_cap,
                needed: self.balance - new_balance,
            });
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof =
            Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        if !signature_proof.is_signed_by(&self.owner) {
            return Err(AccountError::InvalidSignature);
        }

        Ok(())
    }

    fn min_cap(&self, time: u64) -> Coin {
        if self.time_step > 0 && self.step_amount > Coin::ZERO {
            let steps = (time as i128 - self.start_time as i128) / self.time_step as i128;
            let min_cap =
                u64::from(self.total_amount) as i128 - steps * u64::from(self.step_amount) as i128;
            // Since all parameters have been validated, this will be safe as well.
            Coin::from_u64_unchecked(min_cap.max(0) as u64)
        } else {
            Coin::ZERO
        }
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for VestingContract {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;

        tx_logger.push_log(Log::VestingCreate {
            contract_address: transaction.recipient.clone(),
            owner: data.owner.clone(),
            start_time: data.start_time,
            time_step: data.time_step,
            step_amount: data.step_amount,
            total_amount: data.total_amount,
        });

        Ok(Account::Vesting(VestingContract {
            balance: initial_balance + transaction.value,
            owner: data.owner,
            start_time: data.start_time,
            time_step: data.time_step,
            step_amount: data.step_amount,
            total_amount: data.total_amount,
        }))
    }

    fn revert_new_contract(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance -= transaction.value;

        tx_logger.push_log(Log::VestingCreate {
            contract_address: transaction.recipient.clone(),
            owner: self.owner.clone(),
            start_time: self.start_time,
            time_step: self.time_step,
            step_amount: self.step_amount,
            total_amount: self.total_amount,
        });

        Ok(())
    }

    fn commit_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let new_balance = self.balance.safe_sub(transaction.total_value())?;
        self.can_change_balance(transaction, new_balance, block_state)?;
        self.balance = new_balance;

        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::transfer_log(transaction));

        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance += transaction.total_value();

        tx_logger.push_log(Log::transfer_log(transaction));
        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let new_balance = self.balance.safe_sub(transaction.fee)?;
        // XXX This check should not be necessary since are also checking this in reserve_balance()
        self.can_change_balance(transaction, new_balance, block_state)?;
        self.balance = new_balance;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance += transaction.fee;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        block_state: &BlockState,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        let needed = reserved_balance
            .balance()
            .checked_add(transaction.total_value())
            .ok_or(AccountError::InvalidCoinValue)?;
        let new_balance = self.balance.safe_sub(needed)?;
        self.can_change_balance(transaction, new_balance, block_state)?;

        reserved_balance.reserve(self.balance, transaction.total_value())
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        reserved_balance.release(transaction.total_value());
        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountInherentInteraction for VestingContract {
    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForTarget)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForTarget)
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountPruningInteraction for VestingContract {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        Some(PrunedVestingContract::from(self).into())
    }

    fn restore(
        _ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let receipt = pruned_account.ok_or(AccountError::InvalidReceipt)?;
        let pruned_account = PrunedVestingContract::try_from(receipt)?;
        Ok(Account::Vesting(VestingContract::from(pruned_account)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct PrunedVestingContract {
    pub owner: Address,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl From<VestingContract> for PrunedVestingContract {
    fn from(contract: VestingContract) -> Self {
        PrunedVestingContract {
            owner: contract.owner,
            start_time: contract.start_time,
            time_step: contract.time_step,
            step_amount: contract.step_amount,
            total_amount: contract.total_amount,
        }
    }
}

impl From<PrunedVestingContract> for VestingContract {
    fn from(receipt: PrunedVestingContract) -> Self {
        VestingContract {
            balance: Coin::ZERO,
            owner: receipt.owner,
            start_time: receipt.start_time,
            time_step: receipt.time_step,
            step_amount: receipt.step_amount,
            total_amount: receipt.total_amount,
        }
    }
}

convert_receipt!(PrunedVestingContract);
