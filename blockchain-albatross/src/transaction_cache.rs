use std::collections::{HashSet, VecDeque};

use block::Block;
use hash::{Blake2bHash, Hash};
use primitives::policy;

#[derive(Debug, Clone)]
struct BlockDescriptor {
    hash: Blake2bHash,
    prev_hash: Blake2bHash,
    transaction_hashes: Vec<Blake2bHash>,
}

impl<'a> From<&'a Block> for BlockDescriptor {
    fn from(block: &'a Block) -> Self {
        let transactions = block.transactions();

        let hashes = transactions.map(|txs| txs.iter().map(Hash::hash).collect()).unwrap_or_else(|| vec![]);

        BlockDescriptor {
            hash: block.hash(),
            prev_hash: block.parent_hash().clone(),
            transaction_hashes: hashes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionCache {
    transaction_hashes: HashSet<Blake2bHash>,
    block_order: VecDeque<BlockDescriptor>,
}

impl Default for TransactionCache {
    fn default() -> Self {
        TransactionCache {
            transaction_hashes: HashSet::new(),
            block_order: VecDeque::with_capacity(policy::TRANSACTION_VALIDITY_WINDOW as usize),
        }
    }
}

impl TransactionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, transaction_hash: &Blake2bHash) -> bool {
        self.transaction_hashes.contains(&transaction_hash)
    }

    pub fn contains_any(&self, block: &Block) -> bool {
        let transactions = block.transactions();

        if transactions.is_none() {
            return false;
        }

        for transaction in transactions.unwrap().iter() {
            if self.contains(&transaction.hash()) {
                return true;
            }
        }

        false
    }

    pub fn push_block(&mut self, block: &Block) {
        assert!(self.block_order.is_empty() || *block.parent_hash() == self.block_order.back().as_ref().unwrap().hash);

        let descriptor = BlockDescriptor::from(block);
        for hash in &descriptor.transaction_hashes {
            let is_new = self.transaction_hashes.insert(hash.clone());
            assert!(is_new);
        }
        self.block_order.push_back(descriptor);

        if self.block_order.len() as u32 > policy::TRANSACTION_VALIDITY_WINDOW {
            self.shift_block();
        }
    }

    pub fn revert_block(&mut self, block: &Block) {
        let descriptor = self.block_order.pop_back();

        if let Some(descriptor) = descriptor {
            assert_eq!(descriptor.hash, block.hash());

            for hash in &descriptor.transaction_hashes {
                self.transaction_hashes.remove(hash);
            }
        }
    }

    pub fn prepend_block(&mut self, block: &Block) {
        assert!(self.block_order.is_empty() || block.hash() == self.block_order.front().as_ref().unwrap().prev_hash);

        assert!(self.missing_blocks() > 0);

        let descriptor = BlockDescriptor::from(block);

        for hash in &descriptor.transaction_hashes {
            let is_new = self.transaction_hashes.insert(hash.clone());
            assert!(is_new);
        }

        self.block_order.push_front(descriptor);
    }

    fn shift_block(&mut self) {
        let descriptor_opt = self.block_order.pop_front();

        if let Some(descriptor) = descriptor_opt {
            for hash in &descriptor.transaction_hashes {
                self.transaction_hashes.remove(hash);
            }
        }
    }

    pub fn missing_blocks(&self) -> u32 {
        policy::TRANSACTION_VALIDITY_WINDOW - self.block_order.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.block_order.is_empty()
    }

    pub fn head_hash(&self) -> Blake2bHash {
        self.block_order.back().as_ref().unwrap().hash.clone()
    }

    pub fn tail_hash(&self) -> Blake2bHash {
        self.block_order.front().as_ref().unwrap().hash.clone()
    }
}
