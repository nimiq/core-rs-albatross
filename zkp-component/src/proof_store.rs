#[cfg(feature = "database-storage")]
use nimiq_database::{
    traits::{Database, ReadTransaction, WriteTransaction},
    DatabaseProxy, TableProxy,
};

use crate::types::*;

/// Defines an interface for storing and retrieving ZK proofs.
pub trait ProofStore: Send {
    /// Gets a ZK proof.
    fn get_zkp(&self) -> Option<ZKProof>;

    /// Sets or stores a ZK proof.
    fn set_zkp(&self, zk_proof: &ZKProof);
}

#[cfg(feature = "database-storage")]
/// DB implementation of a ProofStore meant for persistent storage
#[derive(Debug)]
pub struct DBProofStore {
    /// Environment for the DB creation and transaction handling.
    env: DatabaseProxy,
    // A database of the current zkp state.
    zkp_db: TableProxy,
}

#[cfg(feature = "database-storage")]
impl DBProofStore {
    const PROOF_DB_NAME: &'static str = "ZKPState";
    const PROOF_KEY: &'static str = "proof";

    pub fn new(env: DatabaseProxy) -> Self {
        let zkp_db = env.open_table(Self::PROOF_DB_NAME.to_string());

        Self { env, zkp_db }
    }
}

#[cfg(feature = "database-storage")]
impl ProofStore for DBProofStore {
    fn get_zkp(&self) -> Option<ZKProof> {
        self.env
            .read_transaction()
            .get(&self.zkp_db, Self::PROOF_KEY)
    }

    fn set_zkp(&self, zk_proof: &ZKProof) {
        let mut tx = self.env.write_transaction();
        tx.put(&self.zkp_db, Self::PROOF_KEY, zk_proof);
        tx.commit();
    }
}
