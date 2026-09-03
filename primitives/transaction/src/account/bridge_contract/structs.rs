use nimiq_keys::Address;
use nimiq_serde::{Deserialize, Serialize};

use super::core::{ChainConfig, OutgoingTransaction};
use crate::{account::htlc_contract::AnyHash, SignatureProof, Transaction, TransactionError};

/// Data used to create bridge contracts.
///
/// Used in [`Transaction::recipient_data`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreationTransactionData {
    /// The owner/operator of the bridge contract
    pub owner: Address,
    /// The oracle contract address for state verification
    pub oracle_address: Address,
    /// Source chain ID this bridge instance supports
    pub source_chain_id: u32,
    /// Chain-specific configuration (address format, endianness, hash function, etc.)
    pub chain_config: ChainConfig,
}

impl CreationTransactionData {
    pub fn parse_data(data: &[u8]) -> Result<Self, TransactionError> {
        Ok(Self::deserialize_all(data)?)
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Self::parse_data(&transaction.recipient_data)
    }

    pub fn verify(&self) -> Result<(), TransactionError> {
        if self.source_chain_id == 0 {
            warn!("Invalid creation data: source_chain_id may not be zero");
            return Err(TransactionError::InvalidData);
        }
        // Verify chain config matches the source_chain_id
        if self.chain_config.chain_id != self.source_chain_id {
            warn!("Chain config chain_id does not match source_chain_id");
            return Err(TransactionError::InvalidData);
        }
        // The release path (extract_burn_transaction_hash and AnyMerkleProof)
        // supports only Blake2b, Sha256 and Keccak256. A bridge configured with any
        // other hash function could accept deposits but never process a burn-proof
        // release, permanently locking user funds. The match is exhaustive so a
        // newly added AnyHash variant forces an explicit support decision here.
        match self.chain_config.hash_function {
            AnyHash::Blake2b(_) | AnyHash::Sha256(_) | AnyHash::Keccak256(_) => {}
            AnyHash::Sha512(_) => {
                warn!("Invalid creation data: Sha512 hash_function is not supported by the bridge release path");
                return Err(TransactionError::InvalidData);
            }
        }
        // Reject programs that can never complete a release. Such a bridge would accept deposits
        // and fail every withdrawal, locking user funds permanently, so the screen belongs at
        // creation time where it is still recoverable.
        for op in &self.chain_config.validation_program.operations {
            match op {
                // Assert and PushExpected* require a ValidationContext (full execution mode) and
                // cause parse_burn_data → extract_only to always fail.
                super::core::ValidationOp::Assert
                | super::core::ValidationOp::PushExpectedAmount
                | super::core::ValidationOp::PushExpectedAddress
                | super::core::ValidationOp::PushExpectedNonce
                | super::core::ValidationOp::PushExpectedValidityHeight => {
                    warn!("Validation program contains an operation incompatible with extraction-only mode");
                    return Err(TransactionError::InvalidData);
                }
                // A zero divisor fails on every burn payload. It is caught here rather than only
                // at runtime because the divisor is an immediate operand and is therefore known
                // at creation time — a stack-supplied one could not be screened statically.
                super::core::ValidationOp::LoadEvmU64Scaled(0) => {
                    warn!("Validation program divides by zero and could never process a release");
                    return Err(TransactionError::InvalidData);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Data for outgoing transactions from bridge contracts (releasing funds).
///
/// Used in [`Transaction::sender_data`] when the bridge contract is the sender
/// and is releasing funds after a burn proof is verified. This handles the path
/// where funds leave the bridge to be released to a user after they burned
/// wrapped tokens on another chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutgoingBridgeTransactionData {
    /// Proof of burn on the source chain (contains all transaction data)
    pub burn_proof: OutgoingTransaction,
    /// Signature proof signed by the bridge owner
    pub proof: SignatureProof,
}

impl OutgoingBridgeTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Self::deserialize_all(&transaction.sender_data)?)
    }

    pub fn set_signature(&mut self, signature_proof: SignatureProof) {
        self.proof = signature_proof;
    }

    pub fn set_signature_on_data(
        data: &[u8],
        signature_proof: SignatureProof,
    ) -> Result<Vec<u8>, nimiq_serde::DeserializeError> {
        let mut data = OutgoingBridgeTransactionData::deserialize_from_vec(data)?;
        data.set_signature(signature_proof);
        Ok(data.serialize_to_vec())
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        // Verify signature by creating a transaction without the signature first
        verify_transaction_signature(transaction, &self.proof)?;
        Ok(())
    }
}

fn verify_transaction_signature(
    transaction: &Transaction,
    sig_proof: &SignatureProof,
) -> Result<(), TransactionError> {
    // If we are verifying the signature on an outgoing transaction, then we need to reset the
    // signature field first.
    let tx = {
        let mut tx_without_sig = transaction.clone();

        tx_without_sig.sender_data = OutgoingBridgeTransactionData::set_signature_on_data(
            &tx_without_sig.sender_data,
            SignatureProof::default(),
        )
        .map_err(|_| TransactionError::InvalidData)?;

        tx_without_sig.serialize_content()
    };

    if !sig_proof.verify(&tx) {
        warn!(?transaction, "Invalid proof");
        return Err(TransactionError::InvalidProof);
    }

    Ok(())
}
