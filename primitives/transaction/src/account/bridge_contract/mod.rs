// Bridge contract module
//
// This module contains all bridge contract functionality for cross-chain asset transfers.
// The main implementation is in bridge_contract.rs, with transaction data structures in structs.rs

use nimiq_primitives::account::AccountType;

use crate::{
    account::AccountTransactionVerification, Transaction, TransactionError, TransactionFlags,
};

// Module declarations
mod core;
pub mod decode_arithmetic;
pub mod structs;

// Re-export everything from core and structs
pub use self::{core::*, structs::*};

/// The verifier trait for a bridge contract. This only uses data available in the transaction.
pub struct BridgeContractVerifier {}

impl AccountTransactionVerification for BridgeContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Bridge);

        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            // Contract creation transaction
            if transaction.recipient != transaction.contract_creation_address() {
                warn!(
                    ?transaction,
                    "Recipient address must match contract creation address",
                );
                return Err(TransactionError::InvalidForRecipient);
            }

            let data = CreationTransactionData::parse(transaction)?;
            data.verify()?;
        } else if transaction.flags.contains(TransactionFlags::SIGNALING) {
            // Signaling transactions are not allowed for incoming to bridge
            warn!(?transaction, "Signaling transactions are not allowed");
            return Err(TransactionError::InvalidForRecipient);
        } else {
            // Regular incoming transaction (user locking funds)
            // No special validation needed - standard transaction rules apply
        }

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Bridge);

        // Parse and verify the outgoing data from sender_data
        let data = OutgoingBridgeTransactionData::parse(transaction)?;
        data.verify(transaction)?;

        // Value must be non-zero (releasing funds)
        if transaction.value.is_zero() {
            warn!(
                ?transaction,
                "Outgoing bridge transactions must have non-zero value",
            );
            return Err(TransactionError::InvalidValue);
        }

        Ok(())
    }
}
