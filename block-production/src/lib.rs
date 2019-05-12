extern crate nimiq_block as block;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_mempool as mempool;
extern crate nimiq_network_primitives as network_primitives;

use std::sync::Arc;

use beserial::Serialize;
use block::{Block, BlockBody, BlockHeader, BlockInterlink, Target};
use blockchain::Blockchain;
use hash::Hash;
use keys::Address;
use mempool::Mempool;
use network_primitives::networks::NetworkInfo;

pub struct BlockProducer<'env> {
    blockchain: Arc<Blockchain<'env>>,
    mempool: Arc<Mempool<'env>>,
}

impl<'env> BlockProducer<'env> {
    pub fn new(blockchain: Arc<Blockchain<'env>>, mempool: Arc<Mempool<'env>>) -> Self {
        BlockProducer { blockchain, mempool }
    }

    pub fn next_block(&self, timestamp: u32, miner: Address, extra_data: Vec<u8>) -> Block {
        // Lock blockchain/mempool while constructing the block.
        let _lock = self.blockchain.push_lock.lock();

        let target = self.blockchain.get_next_target(None);
        let interlink = self.blockchain.head().get_next_interlink(&target);
        let body = self.next_body(interlink.serialized_size(), miner, extra_data);
        let header = self.next_header(target, timestamp, &interlink, &body);

        Block {
            header,
            interlink,
            body: Some(body)
        }
    }

    fn next_body(&self, interlink_size: usize, miner: Address, extra_data: Vec<u8>) -> BlockBody {
        let max_size = Block::MAX_SIZE
            - BlockHeader::SIZE
            - interlink_size
            - BlockBody::get_metadata_size(extra_data.len());
        let mut transactions = self.mempool.get_transactions_for_block(max_size);
        // In v1, inherents never produce receipts, thus we can omit the reward inherent.
        let mut receipts = self.blockchain.state().accounts()
            .collect_receipts(&transactions, &vec![], self.blockchain.height() + 1)
            .expect("Failed to collect receipts during block production");

        let mut size = transactions.iter().fold(0, |size, tx| size + tx.serialized_size())
            + receipts.iter().fold(0, |size, pruned_account| size + pruned_account.serialized_size());
        if size > max_size {
            while size > max_size {
                size -= transactions.pop().serialized_size();
            }
            receipts = self.blockchain.state().accounts()
                .collect_receipts(&transactions, &vec![], self.blockchain.height() + 1)
                .expect("Failed to collect pruned accounts during block production");
        }

        transactions.sort_unstable_by(|a, b| a.cmp_block_order(b));
        receipts.sort_unstable();

        BlockBody {
            miner,
            extra_data,
            transactions,
            receipts,
        }
    }

    fn next_header(&self, target: Target, timestamp: u32, interlink: &BlockInterlink, body: &BlockBody) -> BlockHeader {
        let height = self.blockchain.height() + 1;
        let n_bits = target.into();
        let timestamp = u32::max(timestamp, self.blockchain.head().header.timestamp + 1);

        let prev_hash = self.blockchain.head_hash();
        let genesis_hash = NetworkInfo::from_network_id(self.blockchain.network_id).genesis_hash().clone();
        let interlink_hash = interlink.hash(genesis_hash);
        let body_hash = body.hash();
        let accounts_hash = self.blockchain.state().accounts()
            .hash_with(&body.transactions, &vec![body.get_reward_inherent(height)], height)
            .expect("Failed to compute accounts hash during block production");

        BlockHeader {
            version: Block::VERSION,
            prev_hash,
            interlink_hash,
            body_hash,
            accounts_hash,
            n_bits,
            height,
            timestamp,
            nonce: 0,
        }
    }
}
