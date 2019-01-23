use primitives::block::Block;
use hash::{Hash, Blake2bHash};
use primitives::policy;
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone)]
struct BlockDescriptor {
    hash: Blake2bHash,
    prev_hash: Blake2bHash,
    transaction_hashes: Vec<Blake2bHash>
}

impl<'a> From<&'a Block> for BlockDescriptor {
    fn from(block: &'a Block) -> Self {
        let transactions = &block.body.as_ref().unwrap().transactions;
        let hashes = transactions.iter().map(|tx| tx.hash()).collect();

        BlockDescriptor {
            hash: block.header.hash(),
            prev_hash: block.header.prev_hash.clone(),
            transaction_hashes: hashes
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionCache {
    transaction_hashes: HashSet<Blake2bHash>,
    block_order: VecDeque<BlockDescriptor>
}

impl TransactionCache {
    pub fn new() -> Self {
        return TransactionCache {
            transaction_hashes: HashSet::new(),
            block_order: VecDeque::with_capacity(policy::TRANSACTION_VALIDITY_WINDOW as usize)
        };
    }

    pub fn contains(&self, transaction_hash: &Blake2bHash) -> bool {
        self.transaction_hashes.contains(&transaction_hash)
    }

    pub fn contains_any(&self, block: &Block) -> bool {
        for transaction in block.body.as_ref().unwrap().transactions.iter() {
            if self.contains(&transaction.hash()) {
                return true;
            }
        }
        return false;
    }

    pub fn push_block(&mut self, block: &Block) {
        assert!(self.block_order.is_empty() || block.header.prev_hash == self.block_order.back().as_ref().unwrap().hash);

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
        if descriptor.is_some() {
            for hash in &descriptor.unwrap().transaction_hashes {
                self.transaction_hashes.remove(hash);
            }
        }
    }

    pub fn prepend_block(&mut self, block: &Block) {
        assert!(self.block_order.is_empty() || block.header.hash::<Blake2bHash>() == self.block_order.front().as_ref().unwrap().prev_hash);
        assert!(self.missing_blocks() > 0);

        let descriptor = BlockDescriptor::from(block);
        for hash in &descriptor.transaction_hashes {
            let is_new = self.transaction_hashes.insert(hash.clone());
            assert!(is_new);
        }
        self.block_order.push_front(descriptor);
    }

    fn shift_block(&mut self) {
        let descriptor = self.block_order.pop_front();
        if descriptor.is_some() {
            for hash in &descriptor.unwrap().transaction_hashes {
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
