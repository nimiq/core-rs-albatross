use nimiq_account::{Receipt, Receipts};
use nimiq_block::*;
use nimiq_blockchain::Blockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_primitives::networks::NetworkInfo;
use nimiq_primitives::policy;
use nimiq_transaction::Transaction;

mod blockchain;
mod chain_info;
mod chain_store;
mod super_block_counts;
mod transaction_cache;
#[cfg(feature = "transaction-store")]
mod transaction_store;

pub fn mine_header(header: &mut BlockHeader) {
    println!("Mining block at height {} with difficulty {}", header.height, Difficulty::from(header.n_bits));
    while !header.verify_proof_of_work() {
        header.nonce += 1;
        if header.nonce % 10000 == 0 {
            println!("Mining ... {}", header.nonce);
        }
    }
    println!("Found nonce {} for header {:?}", header.nonce, header);
}

pub fn next_block<'bc>(blockchain: &'bc Blockchain) -> BlockBuilder<'bc> {
    BlockBuilder::new(blockchain)
}

pub struct BlockBuilder<'bc> {
    blockchain: &'bc Blockchain,
    header: BlockHeader,
    interlink: Option<BlockInterlink>,
    body: BlockBody,
}

impl<'bc> BlockBuilder<'bc> {
    pub fn new(blockchain: &'bc Blockchain) -> Self {
        Self {
            blockchain,
            header: BlockHeader {
                version: Block::VERSION,
                prev_hash: [0u8; Blake2bHash::SIZE].into(),
                interlink_hash: [0u8; Blake2bHash::SIZE].into(),
                body_hash: [0u8; Blake2bHash::SIZE].into(),
                accounts_hash: [0u8; Blake2bHash::SIZE].into(),
                n_bits: 0.into(),
                height: 0,
                timestamp: 0,
                nonce: 0
            },
            interlink: None,
            body: BlockBody {
                miner: [0u8; Address::SIZE].into(),
                extra_data: Vec::new(),
                transactions: Vec::new(),
                receipts: Receipts::default(),
            }
        }
    }

    pub fn with_prev_hash(mut self, prev_hash: Blake2bHash) -> Self {
        self.header.prev_hash = prev_hash;
        self
    }

    pub fn with_nbits(mut self, n_bits: TargetCompact) -> Self {
        self.header.n_bits = n_bits;
        self
    }

    pub fn with_height(mut self, height: u32) -> Self {
        self.header.height = height;
        self
    }

    pub fn with_timestamp(mut self, timestamp: u32) -> Self {
        self.header.timestamp = timestamp;
        self
    }

    pub fn with_nonce(mut self, nonce: u32) -> Self {
        self.header.nonce = nonce;
        self
    }

    pub fn with_transactions(mut self, transactions: Vec<Transaction>) -> Self {
        self.body.transactions = transactions;
        self
    }

    pub fn with_receipts(mut self, receipts: Vec<Receipt>) -> Self {
        self.body.receipts.receipts = receipts;
        self
    }

    pub fn with_miner(mut self, miner: Address) -> Self {
        self.body.miner = miner;
        self
    }

    pub fn with_extra_data(mut self, extra_data: Vec<u8>) -> Self {
        self.body.extra_data = extra_data;
        self
    }

    pub fn with_interlink(mut self, interlink: BlockInterlink) -> Self {
        self.interlink = Some(interlink);
        self
    }

    pub fn build(mut self) -> Block {
        let head = self.blockchain.head();
        let next_target = self.blockchain.get_next_target(None);

        if self.header.height == 0 {
            self.header.height = head.header.height + 1;
        }
        if self.header.timestamp == 0 {
            self.header.timestamp = head.header.timestamp + policy::BLOCK_TIME;
        }
        if self.header.n_bits == 0.into() {
            self.header.n_bits = TargetCompact::from(&next_target);
        }
        if self.header.prev_hash == [0u8; Blake2bHash::SIZE].into() {
            self.header.prev_hash = self.blockchain.head_hash();
        }

        self.header.body_hash = self.body.hash();

        if self.interlink.is_none() {
            self.interlink = Some(head.get_next_interlink(&next_target));
        }

        let info = NetworkInfo::from_network_id(self.blockchain.network_id);
        self.header.interlink_hash = self.interlink.as_ref().unwrap().hash(info.genesis_hash().clone());

        // XXX Use default accounts hash if body fails to apply.
        let state = self.blockchain.state();
        let inherents = vec![self.body.get_reward_inherent(self.header.height)];
        self.header.accounts_hash = state.accounts()
            .hash_with(&self.body.transactions, &inherents, self.header.height)
            .unwrap_or([0u8; Blake2bHash::SIZE].into());

        Block {
            header: self.header,
            interlink: self.interlink.unwrap(),
            body: Some(self.body)
        }
    }

    pub fn mine(self) -> Block {
        let mut block = self.build();
        mine_header(&mut block.header);
        block
    }
}
