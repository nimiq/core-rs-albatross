use log::error;

use beserial::Deserialize;
use primitives::account::AccountType;

use crate::account::AccountTransactionVerification;
use crate::SignatureProof;
use crate::{Transaction, TransactionError, TransactionFlags};

/// The verifier trait for a basic account. This only uses data available in the transaction.
pub struct BasicAccountVerifier {}

impl AccountTransactionVerification for BasicAccountVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Basic);

        if transaction.sender == transaction.recipient {
            error!(
                "The following transaction can't have the same sender and recipient:\n{:?}",
                transaction
            );
            return Err(TransactionError::SenderEqualsRecipient);
        }

        if transaction.value.is_zero() {
            error!(
                "The following transaction can't have a zero value:\n{:?}",
                transaction
            );
            return Err(TransactionError::ZeroValue);
        }

        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
            || transaction.flags.contains(TransactionFlags::SIGNALLING)
        {
            error!(
                "Contract creation and signalling not allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Basic);

        // Verify signer & signature.
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.is_signed_by(&transaction.sender)
            || !signature_proof.verify(transaction.serialize_content().as_slice())
        {
            error!(
                "The following transaction has an invalid proof:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}
