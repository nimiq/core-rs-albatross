use beserial::Deserialize;
use primitives::account::AccountType;
use primitives::coin::Coin;
use primitives::policy;

use crate::{Transaction, TransactionError, TransactionFlags};
use crate::account::AccountTransactionVerification;
use crate::SignatureProof;

pub use self::structs::*;

pub mod structs;

pub struct StakingContractVerifier {}

impl AccountTransactionVerification for StakingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Staking);

        if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
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

            // For staking transactions, check minimum stake.
            match data {
                IncomingStakingTransactionData::Stake { .. } => {
                    // TODO: Determine if we want to have a min stake.
                    // If yes, we must enforce it at other points as well (to prevent falling below the min stake).
                    // Implicitly checks that value > 0.
                    if transaction.value < Coin::from_u64_unchecked(policy::MIN_STAKE) {
                        warn!("Stake value below minimum");
                        return Err(TransactionError::InvalidForRecipient);
                    }
                },
                IncomingStakingTransactionData::CreateValidator { .. } => {// Implicitly checks that value > 0.
                    if transaction.value < Coin::from_u64_unchecked(policy::MIN_VALIDATOR_STAKE) {
                        warn!("Validator stake value below minimum");
                        return Err(TransactionError::InvalidForRecipient);
                    }
                },
                _ => {},
            }
        } else {
            if transaction.flags.contains(TransactionFlags::SIGNALLING) {
                warn!("Signalling not allowed for self transactions");
                return Err(TransactionError::InvalidForRecipient);
            }

            let data = SelfStakingTransactionData::parse(transaction)?;
            data.verify(transaction)?;
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
        if transaction.sender == transaction.recipient {
            // Verify signature.
            let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;
            if !signature_proof.verify(transaction.serialize_content().as_slice()) {
                warn!("Invalid signature");
                return Err(TransactionError::InvalidProof);
            }
        } else {
            let proof = OutgoingStakingTransactionProof::parse(transaction)?;
            proof.verify(transaction)?;
        }

        Ok(())
    }
}
