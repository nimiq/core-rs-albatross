use primitives::account::AccountType;

use crate::account::AccountTransactionVerification;
use crate::{Transaction, TransactionError, TransactionFlags};

pub use self::structs::*;

pub mod structs;

pub struct StakingContractVerifier {}

impl AccountTransactionVerification for StakingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Staking);

        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            warn!("Contract creation not allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        // Incoming transactions require the data field to be set correctly
        // and we perform static signature checks here.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        if data.is_signalling() != transaction.flags.contains(TransactionFlags::SIGNALLING) {
            warn!("Signalling must be set for signalling transactions");
            return Err(TransactionError::InvalidForRecipient);
        }

        data.verify(transaction)?;

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Staking);

        let proof = OutgoingStakingTransactionProof::parse(transaction)?;

        proof.verify(transaction)?;

        Ok(())
    }
}
