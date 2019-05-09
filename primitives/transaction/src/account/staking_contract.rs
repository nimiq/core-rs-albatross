use beserial::{Deserialize, ReadBytesExt, Serialize};
use bls::bls12_381::{PublicKey as BlsPublicKey, Signature as BlsSignature};
use keys::Address;
use primitives::account::AccountType;

use crate::{Transaction, TransactionError, TransactionFlags};
use crate::account::AccountTransactionVerification;
use crate::SignatureProof;

pub struct StakingContractVerifier {}

impl AccountTransactionVerification for StakingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Staking);

        if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            warn!("Contract creation not allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.sender != transaction.recipient {
            StakingTransactionData::parse(transaction)?.verify()?;
        }

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Staking);

        // Verify signature.
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;
        if !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StakingTransactionData {
    pub validator_key: BlsPublicKey,
    pub reward_address: Option<Address>,
    pub proof_of_knowledge: BlsSignature,
}

impl StakingTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.data[..];
        let data = Deserialize::deserialize(reader)?;

        // Ensure that transaction data has been fully read.
        if reader.read_u8().is_ok() {
            return Err(TransactionError::InvalidData);
        }

        Ok(data)
    }

    /// Important: Currently, the proof of knowledge of the secret key is a signature of the public key.
    /// If an attacker A ever tricks a validator B into signing a message with content `pk_A - pk_B`,
    /// where `pk_X` is X's BLS public key, A will be able to sign aggregate messages that are valid for
    /// public keys `pk_B + (pk_A - pk_B) = pk_B`.
    /// Alternatives would be to replace the proof of knowledge by a zero-knowledge proof.
    pub fn verify(&self) -> Result<(), TransactionError> {
        if !self.validator_key.verify(&self.validator_key, &self.proof_of_knowledge) {
            return Err(TransactionError::InvalidData)
        }
        Ok(())
    }
}
