use std::collections::{HashMap, HashSet};

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy::Policy;

/// The validity store is used by full nodes to keep track of which
/// transactions have occurred within the validity window without
/// having to store the full transactions
pub struct ValidityStore {
    /// The set of all raw transactions of the current validity window.
    pub(crate) txs: HashSet<Blake2bHash>,

    // A map of txns hashes associated to each block number
    pub(crate) block_txs: HashMap<u32, HashSet<Blake2bHash>>,

    pub(crate) first_bn: u32,

    pub(crate) latest_bn: u32,
}

impl ValidityStore {
    pub(crate) fn new() -> Self {
        Self {
            txs: HashSet::new(),
            block_txs: HashMap::new(),
            first_bn: 0,
            latest_bn: 0,
        }
    }

    pub(crate) fn has_transaction(&self, raw_tx_hash: Blake2bHash) -> bool {
        self.txs.contains(&raw_tx_hash)
    }

    pub(crate) fn _add_block_transactions(&mut self, block: &Block) {
        if let Some(txs) = block.transactions() {
            for tx in txs {
                let raw_tx_hash = tx.raw_tx_hash();

                if let Some(block_txs) = self.block_txs.get_mut(&block.block_number()) {
                    block_txs.insert(raw_tx_hash.clone().into());
                } else {
                    let mut block_txns = HashSet::new();
                    block_txns.insert(raw_tx_hash.clone().into());

                    self.block_txs.insert(block.block_number(), block_txns);
                }

                self.txs.insert(raw_tx_hash.into());
            }
        }
    }

    pub(crate) fn add_transaction(&mut self, block_number: u32, transaction: Blake2bHash) {
        if let Some(block_txs) = self.block_txs.get_mut(&block_number) {
            block_txs.insert(transaction.clone());
        } else {
            let mut block_txns = HashSet::new();
            block_txns.insert(transaction.clone());

            self.block_txs.insert(block_number, block_txns);
        }

        self.txs.insert(transaction);
    }

    // We should only delete blocks from the front or back
    pub(crate) fn delete_block_transactions(&mut self, block_number: u32) {
        log::trace!(bn = block_number, "Deleting block from validity store");

        if self.first_bn == self.latest_bn {
            return;
        }

        // We are deleting from the back
        if block_number == self.first_bn {
            self.first_bn += 1;
        } else if block_number == self.latest_bn {
            self.latest_bn -= 1;
        } else {
            panic!("Trying to remove a block from the middle of the validity store");
        }

        if let Some(block_txns) = self.block_txs.remove(&block_number) {
            for tx in block_txns {
                self.txs.remove(&tx);
            }
        }
    }

    pub(crate) fn prune_validity_store(&mut self) {
        // Compute the number of blocks we currently have in the store
        let mut diff = self.latest_bn - self.first_bn;

        log::trace!(
            first = self.first_bn,
            latest = self.latest_bn,
            diff = diff,
            "Calculated diff for pruning validity store"
        );

        // We only need to keep up to VALIDITY_WINDOW_BLOCKS in the store
        while diff > Policy::transaction_validity_window_blocks() {
            self.delete_block_transactions(self.first_bn);
            diff = self.latest_bn - self.first_bn;
        }
    }

    pub(crate) fn update_validity_store(&mut self, latest_bn: u32) {
        if latest_bn > self.latest_bn {
            self.latest_bn = latest_bn;
        }
        if self.first_bn == 0 {
            self.first_bn = latest_bn;
        }
    }
}
