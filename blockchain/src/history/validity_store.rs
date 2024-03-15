use std::collections::{HashMap, HashSet};

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_transaction::historic_transaction::RawTransactionHash;

/// The validity store is used by full nodes to keep track of which
/// transactions have occurred within the validity window without
/// having to store the full transactions
pub struct ValidityStore {
    /// The set of all raw transactions of the current validity window.
    pub(crate) _txs: HashSet<Blake2bHash>,

    pub(crate) _block_txs: HashMap<u32, HashSet<Blake2bHash>>,
}

impl ValidityStore {
    pub(crate) fn _new() -> Self {
        Self {
            _txs: HashSet::new(),
            _block_txs: HashMap::new(),
        }
    }

    pub(crate) fn _has_transaction(&self, raw_tx_hash: RawTransactionHash) -> bool {
        self._txs.contains(&raw_tx_hash.into())
    }

    pub(crate) fn _add_block_transactions(&mut self, block: &Block) {
        if let Some(txs) = block.transactions() {
            for tx in txs {
                let raw_tx_hash = tx.raw_tx_hash();

                if let Some(block_txs) = self._block_txs.get_mut(&block.block_number()) {
                    block_txs.insert(raw_tx_hash.clone().into());
                } else {
                    let mut block_txns = HashSet::new();
                    block_txns.insert(raw_tx_hash.clone().into());

                    self._block_txs.insert(block.block_number(), block_txns);
                }

                self._txs.insert(raw_tx_hash.into());
            }
        }
    }

    pub(crate) fn _delete_block_transactions(&mut self, block_number: u32) {
        if let Some(block_txns) = self._block_txs.remove(&block_number) {
            for tx in block_txns {
                self._txs.remove(&tx);
            }
        }
    }
}
