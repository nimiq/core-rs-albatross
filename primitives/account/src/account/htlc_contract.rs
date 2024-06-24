use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountType;
use nimiq_primitives::{account::AccountError, coin::Coin};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::htlc_contract::AnyHash;
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::account::htlc_contract::{
    CreationTransactionData, OutgoingHTLCTransactionProof,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, Transaction};

use crate::{convert_receipt, AccountReceipt};
#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
        BlockState,
    },
    reserved_balance::ReservedBalance,
    Account, InherentLogger, Log, TransactionLog,
};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct HashedTimeLockedContract {
    pub balance: Coin,
    pub sender: Address,
    pub recipient: Address,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    #[serde(with = "nimiq_serde::fixint::be")]
    pub timeout: u64,
    pub total_amount: Coin,
}

#[cfg(feature = "interaction-traits")]
impl HashedTimeLockedContract {
    fn can_change_balance(
        &self,
        transaction: &Transaction,
        new_balance: Coin,
        block_state: &BlockState,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        match OutgoingHTLCTransactionProof::deserialize_all(&transaction.proof)? {
            OutgoingHTLCTransactionProof::RegularTransfer {
                hash_depth,
                hash_root,
                pre_image,
                signature_proof,
            } => {
                // Check that the contract has not expired yet.
                if self.timeout < block_state.time {
                    warn!("HTLC has expired: {} < {}", self.timeout, block_state.time);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the provided hash_root is correct.
                if hash_root != self.hash_root {
                    warn!("HTLC hash mismatch");
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the authorized recipient.
                if !signature_proof.is_signed_by(&self.recipient) {
                    return Err(AccountError::InvalidSignature);
                }

                // Check min cap.
                let cap_ratio = 1f64 - (f64::from(hash_depth) / f64::from(self.hash_count));

                let min_cap = Coin::try_from(
                    (cap_ratio * u64::from(self.total_amount) as f64)
                        .floor()
                        .max(0f64) as u64,
                )?;

                if new_balance < min_cap {
                    return Err(AccountError::InsufficientFunds {
                        balance: self.balance - min_cap,
                        needed: self.balance - new_balance,
                    });
                }

                tx_logger.push_log(Log::HTLCRegularTransfer {
                    contract_address: transaction.sender.clone(),
                    pre_image,
                    hash_depth,
                });
            }
            OutgoingHTLCTransactionProof::EarlyResolve {
                signature_proof_recipient,
                signature_proof_sender,
            } => {
                // Check that the transaction is signed by both parties.
                if !signature_proof_recipient.is_signed_by(&self.recipient)
                    || !signature_proof_sender.is_signed_by(&self.sender)
                {
                    return Err(AccountError::InvalidSignature);
                }

                tx_logger.push_log(Log::HTLCEarlyResolve {
                    contract_address: transaction.sender.clone(),
                });
            }
            OutgoingHTLCTransactionProof::TimeoutResolve {
                signature_proof_sender,
            } => {
                // Check that the contract has expired.
                if self.timeout >= block_state.time {
                    warn!(
                        "HTLC has not yet expired: {} >= {}",
                        self.timeout, block_state.time
                    );
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the original sender.
                if !signature_proof_sender.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }

                tx_logger.push_log(Log::HTLCTimeoutResolve {
                    contract_address: transaction.sender.clone(),
                });
            }
        }

        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for HashedTimeLockedContract {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;

        tx_logger.push_log(Log::HTLCCreate {
            contract_address: transaction.recipient.clone(),
            sender: data.sender.clone(),
            recipient: data.recipient.clone(),
            hash_root: data.hash_root.clone(),
            hash_count: data.hash_count,
            timeout: data.timeout,
            total_amount: transaction.value,
        });

        Ok(Account::HTLC(HashedTimeLockedContract {
            balance: initial_balance + transaction.value,
            sender: data.sender,
            recipient: data.recipient,
            hash_root: data.hash_root,
            hash_count: data.hash_count,
            timeout: data.timeout,
            total_amount: transaction.value,
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

        tx_logger.push_log(Log::HTLCCreate {
            contract_address: transaction.recipient.clone(),
            sender: self.sender.clone(),
            recipient: self.recipient.clone(),
            hash_root: self.hash_root.clone(),
            hash_count: self.hash_count,
            timeout: self.timeout,
            total_amount: transaction.value,
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
        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::transfer_log(transaction));

        let new_balance = self.balance.safe_sub(transaction.total_value())?;
        self.can_change_balance(transaction, new_balance, block_state, tx_logger)?;
        self.balance = new_balance;

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

        match OutgoingHTLCTransactionProof::deserialize_all(&transaction.proof)? {
            OutgoingHTLCTransactionProof::RegularTransfer {
                hash_depth,
                pre_image,
                ..
            } => {
                tx_logger.push_log(Log::HTLCRegularTransfer {
                    contract_address: transaction.sender.clone(),
                    pre_image,
                    hash_depth,
                });
            }
            OutgoingHTLCTransactionProof::EarlyResolve { .. } => {
                tx_logger.push_log(Log::HTLCEarlyResolve {
                    contract_address: transaction.sender.clone(),
                });
            }
            OutgoingHTLCTransactionProof::TimeoutResolve { .. } => {
                tx_logger.push_log(Log::HTLCTimeoutResolve {
                    contract_address: transaction.sender.clone(),
                });
            }
        }

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
        self.can_change_balance(
            transaction,
            new_balance,
            block_state,
            &mut TransactionLog::empty(),
        )?;
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
        self.can_change_balance(
            transaction,
            new_balance,
            block_state,
            &mut TransactionLog::empty(),
        )?;

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
impl AccountInherentInteraction for HashedTimeLockedContract {
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
impl AccountPruningInteraction for HashedTimeLockedContract {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        Some(PrunedHashedTimeLockContract::from(self).into())
    }

    fn restore(
        _ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let receipt = pruned_account.ok_or(AccountError::InvalidReceipt)?;
        let pruned_account = PrunedHashedTimeLockContract::try_from(receipt)?;
        Ok(Account::HTLC(HashedTimeLockedContract::from(
            pruned_account,
        )))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PrunedHashedTimeLockContract {
    pub sender: Address,
    pub recipient: Address,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u64,
    pub total_amount: Coin,
}

impl From<HashedTimeLockedContract> for PrunedHashedTimeLockContract {
    fn from(contract: HashedTimeLockedContract) -> Self {
        PrunedHashedTimeLockContract {
            sender: contract.sender,
            recipient: contract.recipient,
            hash_root: contract.hash_root,
            hash_count: contract.hash_count,
            timeout: contract.timeout,
            total_amount: contract.total_amount,
        }
    }
}

impl From<PrunedHashedTimeLockContract> for HashedTimeLockedContract {
    fn from(receipt: PrunedHashedTimeLockContract) -> Self {
        HashedTimeLockedContract {
            balance: Coin::ZERO,
            sender: receipt.sender,
            recipient: receipt.recipient,
            hash_root: receipt.hash_root,
            hash_count: receipt.hash_count,
            timeout: receipt.timeout,
            total_amount: receipt.total_amount,
        }
    }
}

convert_receipt!(PrunedHashedTimeLockContract);
