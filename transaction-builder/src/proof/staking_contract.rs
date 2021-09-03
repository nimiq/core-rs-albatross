use beserial::{Deserialize, Serialize};
use keys::KeyPair;
use transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use transaction::{SignatureProof, Transaction};

use crate::proof::TransactionProofBuilder;

/// The `StakingDataBuilder` can be used to build the data for incoming staking transactions.
/// Such transactions still require a normal proof builder to be used in addition.
///
/// Thus, the [`generate`] method of this proof builder will return another proof builder
/// instead of the final transaction.
#[derive(Clone, Debug)]
pub struct StakingDataBuilder {
    pub transaction: Transaction,
    data: Option<IncomingStakingTransactionData>,
}

impl StakingDataBuilder {
    /// Creates a new `StakingDataBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        StakingDataBuilder {
            transaction,
            data: None,
        }
    }

    /// This method sets the required `signature` proof by signing the transaction
    /// using a key pair.
    pub fn sign_with_key_pair(&mut self, key_pair: &KeyPair) -> &mut Self {
        // Deserialize the data.
        let mut data: IncomingStakingTransactionData =
            Deserialize::deserialize_from_vec(&self.transaction.data[..]).unwrap();

        // If this is a stake transaction, we don't need to sign it.
        match data {
            IncomingStakingTransactionData::Stake { .. } => {}
            _ => {
                let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
                let proof = SignatureProof::from(key_pair.public, signature);
                data.set_signature(proof);
            }
        }

        self.data = Some(data);
        self
    }

    /// This method returns the next proof builder to be used if the staking data signature
    /// has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<TransactionProofBuilder> {
        let mut tx = self.transaction;
        tx.data = self.data?.serialize_to_vec();
        Some(TransactionProofBuilder::without_in_staking(tx))
    }
}

/// The `StakingProofBuilder` can be used to build proofs for transactions
/// to move funds out of the staking contract. These are:
///     - Validator
///         * Drop
///     - Staker
///         * Unstake
///         * Deduct fees
#[derive(Clone, Debug)]
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
    /// from a validator's `key_pair`.
    pub fn drop_validator(&mut self, key_pair: &KeyPair) -> &mut Self {
        let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
        let proof = SignatureProof::from(key_pair.public, signature);
        self.proof = Some(OutgoingStakingTransactionProof::DropValidator { proof });
        self
    }

    /// This methods sets the action to unstake and builds the corresponding proof
    /// from a staker's `key_pair`.
    pub fn unstake(&mut self, key_pair: &KeyPair) -> &mut Self {
        let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
        let proof = SignatureProof::from(key_pair.public, signature);
        self.proof = Some(OutgoingStakingTransactionProof::Unstake { proof });
        self
    }

    /// This methods sets the action to deduct fees and builds the corresponding proof
    /// from a staker's `key_pair`.
    pub fn deduct_fees(&mut self, from_active_balance: bool, key_pair: &KeyPair) -> &mut Self {
        let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
        let proof = SignatureProof::from(key_pair.public, signature);
        self.proof = Some(OutgoingStakingTransactionProof::DeductFees {
            from_active_balance,
            proof,
        });
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
