use beserial::{Deserialize, Serialize};
use bls::KeyPair as BlsKeyPair;
use keys::KeyPair;
use transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use transaction::{SignatureProof, Transaction};

use crate::proof::TransactionProofBuilder;

/// The `SignallingProofBuilder` can be used to build proofs for signalling transactions.
/// Such transactions still require a normal proof builder to be used in addition.
///
/// Thus, the [`generate`] method of this proof builder will return another proof builder
/// instead of the final transaction.
///
/// [`generate`]: struct.SignallingProofBuilder.html#method.generate
pub struct SignallingProofBuilder {
    pub transaction: Transaction,
    data: Option<IncomingStakingTransactionData>,
}

impl SignallingProofBuilder {
    /// Creates a new `SignallingProofBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        SignallingProofBuilder {
            transaction,
            data: None,
        }
    }

    /// This method sets the required signalling `signature` proof by signing the transaction
    /// using a BLS key pair `validator_key_pair`.
    pub fn sign_with_validator_key_pair(&mut self, validator_key_pair: &BlsKeyPair) -> &mut Self {
        // Set validator signature first.
        let mut data: IncomingStakingTransactionData =
            Deserialize::deserialize_from_vec(&self.transaction.data[..]).unwrap();
        let validator_signature =
            validator_key_pair.sign(&self.transaction.serialize_content().as_slice());
        data.set_validator_signature(validator_signature.compress());
        self.data = Some(data);
        self
    }

    /// This method returns the next proof builder to be used if the signalling signature
    /// has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<TransactionProofBuilder> {
        let mut tx = self.transaction;
        tx.data = self.data?.serialize_to_vec();
        Some(TransactionProofBuilder::without_signalling(tx))
    }
}

/// The `StakingProofBuilder` can be used to build proofs for transactions
/// to move funds out of the staking contract.
///
/// These are: unstaking transactions and transactions to drop validators.
pub struct StakingProofBuilder {
    pub transaction: Transaction,
    proof: Option<OutgoingStakingTransactionProof>,
}

impl StakingProofBuilder {
    /// Creates a new `StakingProofBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        StakingProofBuilder {
            transaction,
            proof: None,
        }
    }

    /// This methods sets the action to drop a validator and builds the corresponding proof
    /// from a validator's BLS `key_pair`.
    pub fn drop_validator(&mut self, key_pair: &BlsKeyPair) -> &mut Self {
        let signature = key_pair.sign(&self.transaction.serialize_content().as_slice());
        self.proof = Some(OutgoingStakingTransactionProof::DropValidator {
            validator_key: key_pair.public_key.compress(),
            signature: signature.compress(),
        });
        self
    }

    /// This methods sets the action to unstake and builds the corresponding proof
    /// from a staker's `key_pair`.
    pub fn unstake(&mut self, key_pair: &KeyPair) -> &mut Self {
        let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
        self.proof = Some(OutgoingStakingTransactionProof::Unstake(
            SignatureProof::from(key_pair.public, signature),
        ));
        self
    }

    /// This method generates the final transaction if the proof has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<Transaction> {
        let mut tx = self.transaction;
        tx.proof = self.proof?.serialize_to_vec();
        Some(tx)
    }
}
