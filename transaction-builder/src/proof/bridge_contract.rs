use nimiq_keys::KeyPair;
use nimiq_serde::Serialize;
use nimiq_transaction::{
    account::bridge_contract::OutgoingBridgeTransactionData, SignatureProof, Transaction,
};

/// The `BridgeProofBuilder` can be used to build proofs for transactions that release funds
/// from a bridge contract against a burn proof.
///
/// Releases are permissionless: any key may sign the burn proof, and the account belonging to
/// that key pays the transaction fee.
#[derive(Clone, Debug)]
pub struct BridgeProofBuilder {
    pub transaction: Transaction,
    proof: Option<SignatureProof>,
}

impl BridgeProofBuilder {
    /// Creates a new `BridgeProofBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        BridgeProofBuilder {
            transaction,
            proof: None,
        }
    }

    /// This method sets the required `signature` proof by signing the transaction
    /// using a key pair `key_pair`.
    pub fn sign_with_key_pair(&mut self, key_pair: &KeyPair) -> &mut Self {
        let signature = key_pair.sign(&self.transaction.serialize_content());
        self.proof = Some(SignatureProof::from_ed25519(key_pair.public, signature));
        self
    }

    /// This method generates the final transaction if the proof has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<Transaction> {
        let proof = self.proof?;
        let mut tx = self.transaction;
        // Consensus only checks the proof embedded in `sender_data`. It is also placed in
        // `proof` so the signer, who pays the fee, shows up in the transaction's related addresses.
        tx.sender_data =
            OutgoingBridgeTransactionData::set_signature_on_data(&tx.sender_data, proof.clone())
                .ok()?;
        tx.proof = proof.serialize_to_vec();
        Some(tx)
    }
}
