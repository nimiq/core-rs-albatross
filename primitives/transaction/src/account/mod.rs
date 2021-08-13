use primitives::account::AccountType;

use crate::account::basic_account::BasicAccountVerifier;
use crate::account::htlc_contract::HashedTimeLockedContractVerifier;
use crate::account::staking_contract::StakingContractVerifier;
use crate::account::vesting_contract::VestingContractVerifier;
use crate::{Transaction, TransactionError};

pub mod basic_account;
pub mod htlc_contract;
pub mod staking_contract;
pub mod vesting_contract;

/// Verifies a transaction only using the static data available in the transaction.
/// This is used, for example, to check signatures etc.
/// This particularly does not require an account to exist.
pub trait AccountTransactionVerification: Sized {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError>;
    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError>;
}

impl AccountTransactionVerification for AccountType {
    /// Verifies the incoming part of a transaction only using the static data available in the transaction.
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        match transaction.recipient_type {
            AccountType::Basic => BasicAccountVerifier::verify_incoming_transaction(transaction),
            AccountType::Vesting => {
                VestingContractVerifier::verify_incoming_transaction(transaction)
            }
            AccountType::HTLC => {
                HashedTimeLockedContractVerifier::verify_incoming_transaction(transaction)
            }
            AccountType::Staking => {
                StakingContractVerifier::verify_incoming_transaction(transaction)
            }
            _ => {
                unreachable!()
            }
        }
    }

    /// Verifies the outgoing part of a transaction only using the static data available in the transaction.
    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        match transaction.sender_type {
            AccountType::Basic => BasicAccountVerifier::verify_outgoing_transaction(transaction),
            AccountType::Vesting => {
                VestingContractVerifier::verify_outgoing_transaction(transaction)
            }
            AccountType::HTLC => {
                HashedTimeLockedContractVerifier::verify_outgoing_transaction(transaction)
            }
            AccountType::Staking => {
                StakingContractVerifier::verify_outgoing_transaction(transaction)
            }
            _ => {
                unreachable!()
            }
        }
    }
}
