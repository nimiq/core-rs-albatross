use nimiq_primitives::account::AccountType;

use crate::{
    account::{
        basic_account::BasicAccountVerifier, bridge_contract::BridgeContractVerifier,
        htlc_contract::HashedTimeLockedContractVerifier, oracle_contract::OracleContractVerifier,
        staking_contract::StakingContractVerifier, vesting_contract::VestingContractVerifier,
    },
    Transaction, TransactionError,
};

pub mod basic_account;
pub mod bridge_contract;
pub mod htlc_contract;
pub mod oracle_contract;
pub mod staking_contract;
pub mod vesting_contract;

/// Verifies a transaction only using the static data available in the transaction.
/// This is used, for example, to check signatures etc.
/// This particularly does not require an account to exist.
pub trait AccountTransactionVerification: Sized {
    fn verify_incoming_transaction(
        transaction: &Transaction,
        protocol_version: u16,
    ) -> Result<(), TransactionError>;

    fn verify_outgoing_transaction(
        transaction: &Transaction,
        protocol_version: u16,
    ) -> Result<(), TransactionError>;
}

impl AccountTransactionVerification for AccountType {
    /// Verifies the incoming part of a transaction only using the static data available in the transaction.
    fn verify_incoming_transaction(
        transaction: &Transaction,
        protocol_version: u16,
    ) -> Result<(), TransactionError> {
        match transaction.recipient_type {
            AccountType::Basic => {
                BasicAccountVerifier::verify_incoming_transaction(transaction, protocol_version)
            }
            AccountType::Vesting => {
                VestingContractVerifier::verify_incoming_transaction(transaction, protocol_version)
            }
            AccountType::HTLC => HashedTimeLockedContractVerifier::verify_incoming_transaction(
                transaction,
                protocol_version,
            ),
            AccountType::Staking => {
                StakingContractVerifier::verify_incoming_transaction(transaction, protocol_version)
            }
            AccountType::Oracle => {
                OracleContractVerifier::verify_incoming_transaction(transaction, protocol_version)
            }
            AccountType::Bridge => {
                BridgeContractVerifier::verify_incoming_transaction(transaction, protocol_version)
            }
        }
    }

    /// Verifies the outgoing part of a transaction only using the static data available in the transaction.
    fn verify_outgoing_transaction(
        transaction: &Transaction,
        protocol_version: u16,
    ) -> Result<(), TransactionError> {
        match transaction.sender_type {
            AccountType::Basic => {
                BasicAccountVerifier::verify_outgoing_transaction(transaction, protocol_version)
            }
            AccountType::Vesting => {
                VestingContractVerifier::verify_outgoing_transaction(transaction, protocol_version)
            }
            AccountType::HTLC => HashedTimeLockedContractVerifier::verify_outgoing_transaction(
                transaction,
                protocol_version,
            ),
            AccountType::Staking => {
                StakingContractVerifier::verify_outgoing_transaction(transaction, protocol_version)
            }
            AccountType::Oracle => {
                OracleContractVerifier::verify_outgoing_transaction(transaction, protocol_version)
            }
            AccountType::Bridge => {
                BridgeContractVerifier::verify_outgoing_transaction(transaction, protocol_version)
            }
        }
    }
}
