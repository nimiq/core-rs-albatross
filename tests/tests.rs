extern crate curve25519_dalek;
extern crate beserial;
extern crate ed25519_dalek;
extern crate hex;
extern crate nimiq;
extern crate sha2;
extern crate bigdecimal;
extern crate num_traits;
extern crate num_bigint;
extern crate pretty_env_logger;

use nimiq::consensus::base::account::PrunedAccount;
use nimiq::consensus::base::block::{Block, BlockHeader, BlockBody, Difficulty, TargetCompact};
use nimiq::consensus::base::blockchain::Blockchain;
use nimiq::consensus::base::primitive::Address;
use nimiq::consensus::base::primitive::hash::{Hash, Blake2bHash};
use nimiq::consensus::base::transaction::Transaction;
use nimiq::consensus::policy;

mod consensus;
mod network;
mod utils;

pub fn setup() {
    pretty_env_logger::try_init().unwrap_or(());
}

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

pub fn next_block<'env, 'bc>(blockchain: &'bc Blockchain<'env>) -> BlockBuilder<'env, 'bc> {
    BlockBuilder::new(blockchain)
}

pub struct BlockBuilder<'env, 'bc> {
    blockchain: &'bc Blockchain<'env>,
    header: BlockHeader,
    body: BlockBody,
}

impl<'env, 'bc> BlockBuilder<'env, 'bc> {
    pub fn new(blockchain: &'bc Blockchain<'env>) -> Self {
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
            body: BlockBody {
                miner: [0u8; Address::SIZE].into(),
                extra_data: Vec::new(),
                transactions: Vec::new(),
                pruned_accounts: Vec::new()
            }
        }
    }

    pub fn with_difficulty(mut self, difficulty: Difficulty) -> Self {
        self.header.n_bits = difficulty.into();
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

    pub fn with_pruned_accounts(mut self, pruned_accounts: Vec<PrunedAccount>) -> Self {
        self.body.pruned_accounts = pruned_accounts;
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

        self.header.prev_hash = self.blockchain.head_hash();
        self.header.body_hash = self.body.hash();

        let interlink = head.get_next_interlink(&next_target);
        self.header.interlink_hash = interlink.hash(self.blockchain.network_id);

        // XXX Use default accounts hash if body fails to apply.
        let accounts = self.blockchain.accounts();
        self.header.accounts_hash = accounts
            .hash_with_block_body(&self.body, self.header.height)
            .unwrap_or([0u8; Blake2bHash::SIZE].into());

        Block {
            header: self.header,
            interlink,
            body: Some(self.body)
        }
    }

    pub fn mine(self) -> Block {
        let mut block = self.build();
        mine_header(&mut block.header);
        block
    }
}
