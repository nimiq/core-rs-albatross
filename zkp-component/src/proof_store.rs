#[cfg(feature = "database-storage")]
use nimiq_database::{Database, Environment, ReadTransaction, WriteTransaction};

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
    env: Environment,
    // A database of the current zkp state.
    zkp_db: Database,
}

#[cfg(feature = "database-storage")]
impl DBProofStore {
    const PROOF_DB_NAME: &'static str = "ZKPState";
    const PROOF_KEY: &'static str = "proof";

    pub fn new(env: Environment) -> Self {
        let zkp_db = env.open_database(Self::PROOF_DB_NAME.to_string());

        Self { env, zkp_db }
    }
}

#[cfg(feature = "database-storage")]
impl ProofStore for DBProofStore {
    fn get_zkp(&self) -> Option<ZKProof> {
        ReadTransaction::new(&self.env).get(&self.zkp_db, Self::PROOF_KEY)
    }

    fn set_zkp(&self, zk_proof: &ZKProof) {
        let mut tx = WriteTransaction::new(&self.env);
        tx.put(&self.zkp_db, Self::PROOF_KEY, zk_proof);
        tx.commit();
    }
}
