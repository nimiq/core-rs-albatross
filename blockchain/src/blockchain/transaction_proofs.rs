use std::collections::HashSet;

use database::ReadTransaction;
use hash::Blake2bHash;
use hash::Hash;
use keys::Address;
use tree_primitives::accounts_proof::AccountsProof;
use transaction::TransactionsProof;
use utils::merkle::Blake2bMerkleProof;

use crate::Blockchain;

impl<'env> Blockchain<'env> {
    pub fn get_transactions_proof(&self, block_hash: &Blake2bHash, addresses: &HashSet<Address>) -> Option<TransactionsProof> {
        let block = self.get_block(block_hash, /*include_forks*/ false, /*include_body*/ true)?;
        let body = block.body.as_ref()?;

        let mut matches = Vec::new();
        for transaction in body.transactions.iter() {
            if addresses.contains(&transaction.sender) || addresses.contains(&transaction.recipient) {
                matches.push(transaction.clone());
            }
        }

        let merkle_leaves = body.get_merkle_leaves::<Blake2bHash>();
        let matching_hashes: Vec<Blake2bHash> = matches.iter().map(Hash::hash).collect();
        let proof = Blake2bMerkleProof::new(&merkle_leaves, &matching_hashes);
        Some(TransactionsProof {
            transactions: matches,
            proof,
        })
    }

    pub fn get_accounts_proof(&self, block_hash: &Blake2bHash, addresses: &[Address]) -> Option<AccountsProof> {
        let state = self.state.read();
        // We only support accounts proofs for the head hash.
        if block_hash != &state.head_hash {
            return None;
        }
        let txn = ReadTransaction::new(self.env);
        Some(state.accounts.get_accounts_proof(&txn, addresses))
    }
}
