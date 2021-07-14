use beserial::Deserialize;
use primitives::account::AccountType;

use crate::account::AccountTransactionVerification;
use crate::SignatureProof;
use crate::{Transaction, TransactionError, TransactionFlags};

pub struct BasicAccountVerifier {}

impl AccountTransactionVerification for BasicAccountVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Basic);

        if transaction.sender == transaction.recipient {
            return Err(TransactionError::SenderEqualsRecipient);
        }

        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
            || transaction.flags.contains(TransactionFlags::SIGNALLING)
        {
            warn!("Contract creation and signalling not allowed");
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
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}
