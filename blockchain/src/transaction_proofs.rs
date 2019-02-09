use std::collections::HashSet;

use hash::Blake2bHash;
use hash::Hash;
use keys::Address;
use primitives::transaction::TransactionsProof;
use utils::merkle::Blake2bMerkleProof;

use crate::Blockchain;

impl<'env> Blockchain<'env> {
    pub fn get_transactions_proof(&self, block_hash: &Blake2bHash, addresses: &HashSet<Address>) -> Option<TransactionsProof> {
        let block = self.get_block(block_hash, /*include_forks*/ false, /*include_body*/ true);
        if let Some(ref body) = block.and_then(|block| block.body) {
            let mut matches = Vec::new();
            for transaction in body.transactions.iter() {
                if addresses.contains(&transaction.sender) || addresses.contains(&transaction.recipient) {
                    matches.push(transaction.clone());
                }
            }

            let merkle_leaves = body.get_merkle_leaves::<Blake2bHash>();
            let matching_hashes: Vec<Blake2bHash> = matches.iter().map(|tx| tx.hash()).collect();
            let proof = Blake2bMerkleProof::new(merkle_leaves, matching_hashes);
            return Some(TransactionsProof {
                transactions: matches,
                proof,
            });
        }

        None
    }
}