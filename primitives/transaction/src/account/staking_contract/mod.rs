use beserial::Deserialize;
use primitives::account::AccountType;
use primitives::coin::Coin;

use crate::account::AccountTransactionVerification;
use crate::SignatureProof;
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

        // There are two cases of incoming transactions:
        // Either self transactions (used for stakers) or real incoming transactions.
        // Self transactions only need checks on the sender side.
        // Real incoming transactions require the data field to be set correctly
        // and we perform static signature checks here.
        if transaction.sender != transaction.recipient {
            let data = IncomingStakingTransactionData::parse(transaction)?;

            data.verify(transaction)?;

            if data.is_signalling() != transaction.flags.contains(TransactionFlags::SIGNALLING) {
                warn!("Signalling must be set for signalling transactions");
                return Err(TransactionError::InvalidForRecipient);
            }
        } else {
            let data = SelfStakingTransactionData::parse(transaction)?;

            data.verify(transaction)?;

            if transaction.flags.contains(TransactionFlags::SIGNALLING) {
                warn!("Signalling not allowed for self transactions");
                return Err(TransactionError::InvalidForRecipient);
            }
        }

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Staking);

        // Check that value > 0.
        if transaction.value == Coin::ZERO {
            return Err(TransactionError::ZeroValue);
        }

        // Again, we have two types of transactions here:
        // Self transactions, which require a SignatureProof check and real outgoing transactions
        // with a special proof field structure.
        if transaction.sender != transaction.recipient {
            let proof = OutgoingStakingTransactionProof::parse(transaction)?;

            proof.verify(transaction)?;
        } else {
            // Verify signature.
            let signature_proof: SignatureProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            if !signature_proof.verify(transaction.serialize_content().as_slice()) {
                warn!("Invalid signature");
                return Err(TransactionError::InvalidProof);
            }
        }

        Ok(())
    }
}
