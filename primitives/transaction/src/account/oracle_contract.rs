use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_serde::{Deserialize, DeserializeError, Serialize};

use crate::{
    account::{htlc_contract::AnyHash, AccountTransactionVerification},
    SignatureProof, Transaction, TransactionError, TransactionFlags,
};

/// The verifier trait for an oracle contract. This only uses data available in the transaction.
pub struct OracleContractVerifier {}

impl AccountTransactionVerification for OracleContractVerifier {
    fn verify_incoming_transaction(
        transaction: &Transaction,
        _protocol_version: u16,
    ) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Oracle);

        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            // Contract creation transaction
            if transaction.recipient != transaction.contract_creation_address() {
                warn!(
                    "Recipient address must match contract creation address for the following transaction:\n{:?}",
                    transaction
                );
                return Err(TransactionError::InvalidForRecipient);
            }

            let data = CreationTransactionData::parse(transaction)?;
            data.verify()?;
        } else if transaction.flags.contains(TransactionFlags::SIGNALING) {
            // Signaling transaction for updates or owner changes
            let data = IncomingOracleTransactionData::parse(transaction)?;
            data.verify(transaction)?;
        } else {
            warn!(
                "Only contract creation or signaling transactions are allowed for the following transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        Ok(())
    }

    fn verify_outgoing_transaction(
        transaction: &Transaction,
        _protocol_version: u16,
    ) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Oracle);

        if !transaction.sender_data.is_empty() {
            warn!(
                "The following transaction can't have sender data:\n{:?}",
                transaction
            );
            return Err(TransactionError::Overflow);
        }

        // Verify signature.
        let signature_proof = SignatureProof::deserialize_all(&transaction.proof)?;

        if !signature_proof.verify(&transaction.serialize_content()) {
            warn!("Invalid signature for this transaction:\n{:?}", transaction);
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}

/// Data used to create oracle contracts.
///
/// Used in [`Transaction::recipient_data`].
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreationTransactionData {
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,
    /// The number of hashes that can be stored. This determines the required deposit.
    pub hash_count: u16,
}

impl CreationTransactionData {
    pub fn parse_data(data: &[u8]) -> Result<Self, TransactionError> {
        Ok(Self::deserialize_all(data)?)
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Self::parse_data(&transaction.recipient_data)
    }

    pub fn verify(&self) -> Result<(), TransactionError> {
        if self.hash_count == 0 {
            warn!("Invalid creation data: hash_count may not be zero");
            return Err(TransactionError::InvalidData);
        }
        Ok(())
    }
}

/// Data used for incoming transactions to oracle contracts (updates and owner changes).
///
/// Used in [`Transaction::recipient_data`] for signaling transactions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum IncomingOracleTransactionData {
    /// Update operation to store hashes
    Update {
        /// The hashes to store. The number of hashes must not exceed the contract's hash_count.
        hashes: Vec<AnyHash>,
        /// Signature proof signed by the owner
        proof: SignatureProof,
    },
    /// Change the owner of the contract
    ChangeOwner {
        /// The new owner address
        new_owner: Address,
        /// Signature proof signed by the current owner
        proof: SignatureProof,
    },
}

impl IncomingOracleTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Self::deserialize_all(&transaction.recipient_data)?)
    }

    pub fn set_signature(&mut self, signature_proof: SignatureProof) {
        match self {
            IncomingOracleTransactionData::Update { proof, .. }
            | IncomingOracleTransactionData::ChangeOwner { proof, .. } => {
                *proof = signature_proof;
            }
        }
    }

    pub fn set_signature_on_data(
        data: &[u8],
        signature_proof: SignatureProof,
    ) -> Result<Vec<u8>, DeserializeError> {
        let mut data = IncomingOracleTransactionData::deserialize_from_vec(data)?;
        data.set_signature(signature_proof);
        Ok(data.serialize_to_vec())
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            IncomingOracleTransactionData::Update { proof, .. } => {
                // Verify signature by creating a transaction without the signature first
                verify_transaction_signature(transaction, proof)?;
            }
            IncomingOracleTransactionData::ChangeOwner { proof, .. } => {
                // Verify signature by creating a transaction without the signature first
                verify_transaction_signature(transaction, proof)?;
            }
        }
        Ok(())
    }
}

fn verify_transaction_signature(
    transaction: &Transaction,
    sig_proof: &SignatureProof,
) -> Result<(), TransactionError> {
    // If we are verifying the signature on an incoming transaction, then we need to reset the
    // signature field first.
    let tx = {
        let mut tx_without_sig = transaction.clone();

        tx_without_sig.recipient_data = IncomingOracleTransactionData::set_signature_on_data(
            &tx_without_sig.recipient_data,
            SignatureProof::default(),
        )
        .map_err(|_| TransactionError::InvalidData)?;

        tx_without_sig.serialize_content()
    };

    if !sig_proof.verify(&tx) {
        warn!(
            "Invalid proof. The offending transaction is the following:\n{:?}",
            transaction
        );
        return Err(TransactionError::InvalidProof);
    }

    Ok(())
}
