use log::error;
use nimiq_primitives::account::AccountType;
use nimiq_serde::Deserialize;

use crate::{
    account::AccountTransactionVerification, SignatureProof, Transaction, TransactionError,
    TransactionFlags,
};

/// The verifier trait for a basic account. This only uses data available in the transaction.
pub struct BasicAccountVerifier {}

impl AccountTransactionVerification for BasicAccountVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Basic);

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
            || transaction.flags.contains(TransactionFlags::SIGNALING)
        {
            error!(
                "Contract creation and signaling not allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Basic);

        // Verify signer & signature.
        let signature_proof = SignatureProof::deserialize_all(&transaction.proof)?;

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
