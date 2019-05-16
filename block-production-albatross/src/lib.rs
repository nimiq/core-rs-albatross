extern crate nimiq_block_albatross as block;
extern crate nimiq_blockchain_albatross as blockchain;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_mempool as mempool;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_bls as bls;
extern crate nimiq_primitives as primitives;

use std::sync::Arc;
use std::convert::TryFrom;

use beserial::Serialize;
use block::{Block, MicroBlock, PbftProposal, MacroHeader, MicroExtrinsics, MacroExtrinsics, MicroHeader, ViewChangeProof};
use block::ForkProof;
use blockchain::blockchain::Blockchain;
use hash::Hash;
use mempool::Mempool;
use bls::bls12_381::{SecretKey, KeyPair};
use block::MicroJustification;
use primitives::coin::Coin;

pub struct BlockProducer<'env> {
    pub blockchain: Arc<Blockchain<'env>>,
    pub mempool: Arc<Mempool<'env, Blockchain<'env>>>,
    pub validator_key: KeyPair,
}

impl<'env> BlockProducer<'env> {
    pub fn new(blockchain: Arc<Blockchain<'env>>, mempool: Arc<Mempool<'env, Blockchain<'env>>>, validator_key: KeyPair) -> Self {
        BlockProducer { blockchain, mempool, validator_key }
    }

    pub fn next_macro_block_proposal(&self, timestamp: u64, view_change_proof: Option<ViewChangeProof>) -> PbftProposal {
        // TODO: Lock blockchain/mempool while constructing the block.
        // let _lock = self.blockchain.push_lock.lock();

        let extrinsics = self.next_macro_extrinsics();
        let header = self.next_macro_header(timestamp, &extrinsics);

        PbftProposal {
            header,
            view_change: view_change_proof,
        }
    }

    pub fn next_micro_block(&self, fork_proofs: Vec<ForkProof>, timestamp: u64, view_number: u32, extra_data: Vec<u8>, view_change_proof: Option<ViewChangeProof>) -> MicroBlock {
        // TODO: Lock blockchain/mempool while constructing the block.
        // let _lock = self.blockchain.push_lock.lock();

        let extrinsics = self.next_micro_extrinsics(fork_proofs, extra_data);
        let header = self.next_micro_header(timestamp, view_number, &extrinsics);
        let signature = self.validator_key.sign(&header).compress();

        MicroBlock {
            header,
            extrinsics: Some(extrinsics),
            justification: MicroJustification {
                signature,
                view_change_proof,
            },
        }
    }

    pub fn next_macro_extrinsics(&self) -> MacroExtrinsics {
        self.blockchain.next_slots().into()
    }

    fn next_micro_extrinsics(&self, fork_proofs: Vec<ForkProof>, extra_data: Vec<u8>) -> MicroExtrinsics {
        let max_size = MicroBlock::MAX_SIZE
            - MicroHeader::SIZE
            - MicroExtrinsics::get_metadata_size(fork_proofs.len(), extra_data.len());
        let mut transactions = self.mempool.get_transactions_for_block(max_size);

        let inherents = self.blockchain.create_slash_inherents(&fork_proofs, /* TODO */ None);

        let mut receipts = self.blockchain.state().accounts()
            .collect_receipts(&transactions, &inherents, self.blockchain.height() + 1)
            .expect("Failed to collect receipts during block production");

        let mut size = transactions.iter().fold(0, |size, tx| size + tx.serialized_size())
            + receipts.iter().fold(0, |size, receipt| size + receipt.serialized_size());
        if size > max_size {
            while size > max_size {
                size -= transactions.pop().serialized_size();
            }
            receipts = self.blockchain.state().accounts()
                .collect_receipts(&transactions, &inherents, self.blockchain.height() + 1)
                .expect("Failed to collect pruned accounts during block production");
        }

        transactions.sort_unstable_by(|a, b| a.cmp_block_order(b));
        receipts.sort_unstable();

        MicroExtrinsics {
            fork_proofs,
            extra_data,
            transactions,
            receipts,
        }
    }

    pub fn next_macro_header(&self, timestamp: u64, extrinsics: &MacroExtrinsics) -> MacroHeader {
        let block_number = self.blockchain.height() + 1;
        let view_number = self.blockchain.view_number();
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp() + 1);

        let parent_hash = self.blockchain.head_hash();
        let parent_macro_hash = self.blockchain.macro_head_hash();
        let extrinsics_root = extrinsics.hash();

        let validators = self.blockchain.next_validators().into();

        let inherents = self.blockchain.finalize_last_epoch();
        // Rewards are distributed with delay.
        let state_root = self.blockchain.state().accounts()
            .hash_with(&vec![], &inherents, block_number)
            .expect("Failed to compute accounts hash during block production");

        let seed = self.validator_key.sign(self.blockchain.head().seed()).compress();

        MacroHeader {
            version: Block::VERSION,
            validators,
            block_number,
            view_number,
            parent_macro_hash,
            seed,
            parent_hash,
            state_root,
            extrinsics_root,
            timestamp,
        }
    }

    fn next_micro_header(&self, timestamp: u64, view_number: u32, extrinsics: &MicroExtrinsics) -> MicroHeader {
        let block_number = self.blockchain.height() + 1;
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp() + 1);

        let parent_hash = self.blockchain.head_hash();
        let extrinsics_root = extrinsics.hash();

        let inherents = self.blockchain.create_slash_inherents(&extrinsics.fork_proofs, /* TODO */ None);
        // Rewards are distributed with delay.
        let state_root = self.blockchain.state().accounts()
            .hash_with(&extrinsics.transactions, &inherents, block_number)
            .expect("Failed to compute accounts hash during block production");

        let seed = self.validator_key.sign(self.blockchain.head().seed()).compress();

        MicroHeader {
            version: Block::VERSION,
            block_number,
            view_number,
            parent_hash,
            extrinsics_root,
            state_root,
            seed,
            timestamp,
        }
    }
}
