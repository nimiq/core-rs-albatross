use nimiq_keys::KeyPair;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::oracle_contract::IncomingOracleTransactionData, SignatureProof, Transaction,
};

use crate::proof::TransactionProofBuilder;

/// The `OracleDataBuilder` can be used to build the data for signaling transactions to an
/// existing oracle contract (hash updates and owner changes).
///
/// The data is signed by the oracle owner, while the transaction itself is signed by the account
/// paying the fee. Such transactions therefore still require a normal proof builder, which is why
/// the [`generate`](OracleDataBuilder::generate) method returns another proof builder instead of
/// the final transaction.
#[derive(Clone, Debug)]
pub struct OracleDataBuilder {
    pub transaction: Transaction,
    data: Option<IncomingOracleTransactionData>,
}

impl OracleDataBuilder {
    /// Creates a new `OracleDataBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        OracleDataBuilder {
            transaction,
            data: None,
        }
    }

    /// This method sets the required `signature` proof by signing the transaction
    /// using the key pair of the oracle owner.
    pub fn sign_with_key_pair(&mut self, key_pair: &KeyPair) -> &mut Self {
        // On malformed recipient_data, leave `self.data` as `None` so `generate()` returns `None`.
        let Ok(mut data) =
            IncomingOracleTransactionData::deserialize_all(&self.transaction.recipient_data)
        else {
            return self;
        };

        let signature = key_pair.sign(&self.transaction.serialize_content());
        data.set_signature(SignatureProof::from_ed25519(key_pair.public, signature));

        self.data = Some(data);
        self
    }

    /// This method returns the next proof builder to be used if the oracle data signature
    /// has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<TransactionProofBuilder> {
        let mut tx = self.transaction;
        tx.recipient_data = self.data?.serialize_to_vec();
        Some(TransactionProofBuilder::for_sender(tx))
    }
}
