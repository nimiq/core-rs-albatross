#[cfg(feature = "database-storage")]
use nimiq_database::{
    declare_table,
    mdbx::MdbxDatabase,
    traits::{Database, ReadTransaction, WriteTransaction},
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
declare_table!(ZKProofTable, "ZKPState", () => ZKProof);

#[cfg(feature = "database-storage")]
/// DB implementation of a ProofStore meant for persistent storage
#[derive(Debug)]
pub struct DBProofStore {
    /// Environment for the DB creation and transaction handling.
    env: MdbxDatabase,
}

#[cfg(feature = "database-storage")]
impl DBProofStore {
    pub fn new(env: MdbxDatabase) -> Self {
        env.create_regular_table(&ZKProofTable);

        Self { env }
    }
}

#[cfg(feature = "database-storage")]
impl ProofStore for DBProofStore {
    fn get_zkp(&self) -> Option<ZKProof> {
        self.env.read_transaction().get(&ZKProofTable, &())
    }

    fn set_zkp(&self, zk_proof: &ZKProof) {
        let mut tx = self.env.write_transaction();
        tx.put(&ZKProofTable, &(), zk_proof);
        tx.commit();
    }
}
