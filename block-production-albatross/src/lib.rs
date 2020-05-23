extern crate nimiq_block_albatross as block;
extern crate nimiq_blockchain_albatross as blockchain;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_database as database;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_mempool as mempool;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;
extern crate nimiq_vrf as vrf;

use std::sync::Arc;

use beserial::Serialize;
use block::ForkProof;
use block::MicroJustification;
use block::{
    Block, MacroBlock, MacroExtrinsics, MacroHeader, MicroBlock, MicroExtrinsics, MicroHeader,
    PbftProposal, ViewChangeProof, ViewChanges,
};
use blockchain::blockchain::Blockchain;
use blockchain::reward_registry::SlashedSetSelector;
use blockchain_base::AbstractBlockchain;
use bls::KeyPair;
use database::WriteTransaction;
use hash::{Blake2bHash, Hash};
use mempool::Mempool;
use primitives::policy;
use primitives::slot::ValidatorSlots;
use vrf::VrfSeed;

pub struct BlockProducer {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Option<Arc<Mempool<Blockchain>>>,
    pub validator_key: KeyPair,
}

impl BlockProducer {
    pub fn new(
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool<Blockchain>>,
        validator_key: KeyPair,
    ) -> Self {
        BlockProducer {
            blockchain,
            mempool: Some(mempool),
            validator_key,
        }
    }

    pub fn new_without_mempool(blockchain: Arc<Blockchain>, validator_key: KeyPair) -> Self {
        BlockProducer {
            blockchain,
            mempool: None,
            validator_key,
        }
    }

    /// Needs to be called with the Blockchain lock held.
    pub fn next_macro_block_proposal(
        &self,
        timestamp: u64,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
    ) -> (PbftProposal, MacroExtrinsics) {
        let seed = self
            .blockchain
            .head()
            .seed()
            .sign_next(&self.validator_key.secret);
        let mut txn = self.blockchain.write_transaction();

        let mut header = self.next_macro_header(&mut txn, timestamp, view_number, &seed);
        let extrinsics = self.next_macro_extrinsics(&mut txn, &seed);
        header.extrinsics_root = extrinsics.hash();

        txn.abort();

        (
            PbftProposal {
                header,
                view_change: view_change_proof,
            },
            extrinsics,
        )
    }

    /// Needs to be called with the Blockchain lock held.
    pub fn next_micro_block(
        &self,
        fork_proofs: Vec<ForkProof>,
        timestamp: u64,
        view_number: u32,
        extra_data: Vec<u8>,
        view_change_proof: Option<ViewChangeProof>,
    ) -> MicroBlock {
        let view_changes = ViewChanges::new(
            self.blockchain.block_number() + 1,
            self.blockchain.next_view_number(),
            view_number,
        );
        let extrinsics = self.next_micro_extrinsics(fork_proofs, extra_data, &view_changes);
        let header = self.next_micro_header(timestamp, view_number, &extrinsics, &view_changes);
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

    pub fn next_macro_extrinsics(
        &self,
        txn: &mut WriteTransaction,
        seed: &VrfSeed,
    ) -> MacroExtrinsics {
        // Determine slashed set without txn, so that it is not garbage collected yet.
        let prev_epoch = policy::epoch_at(self.blockchain.height() + 1) - 1;
        // Select whole slashed set here.
        let slashed_set = self.blockchain.state().reward_registry().slashed_set(
            prev_epoch,
            SlashedSetSelector::All,
            None,
        );
        MacroExtrinsics::from_slashed_set(slashed_set)
    }

    fn next_micro_extrinsics(
        &self,
        fork_proofs: Vec<ForkProof>,
        extra_data: Vec<u8>,
        view_changes: &Option<ViewChanges>,
    ) -> MicroExtrinsics {
        let max_size = MicroBlock::MAX_SIZE
            - MicroHeader::SIZE
            - MicroExtrinsics::get_metadata_size(fork_proofs.len(), extra_data.len());
        let mut transactions = self
            .mempool
            .as_ref()
            .map(|mempool| mempool.get_transactions_for_block(max_size))
            .unwrap_or_else(Vec::new);

        let inherents = self
            .blockchain
            .create_slash_inherents(&fork_proofs, view_changes, None);

        self.blockchain
            .state()
            .accounts()
            .collect_receipts(&transactions, &inherents, self.blockchain.height() + 1)
            .expect("Failed to collect receipts during block production");

        let mut size = transactions
            .iter()
            .fold(0, |size, tx| size + tx.serialized_size());
        if size > max_size {
            while size > max_size {
                size -= transactions.pop().serialized_size();
            }
            self.blockchain
                .state()
                .accounts()
                .collect_receipts(&transactions, &inherents, self.blockchain.height() + 1)
                .expect("Failed to collect pruned accounts during block production");
        }

        transactions.sort_unstable_by(|a, b| a.cmp_block_order(b));

        MicroExtrinsics {
            fork_proofs,
            extra_data,
            transactions,
        }
    }

    pub fn next_macro_header(
        &self,
        txn: &mut WriteTransaction,
        timestamp: u64,
        view_number: u32,
        seed: &VrfSeed,
    ) -> MacroHeader {
        let block_number = self.blockchain.height() + 1;
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp() + 1);

        let parent_hash = self.blockchain.head_hash();
        let parent_macro_hash = self.blockchain.macro_head_hash();

        let mut header = MacroHeader {
            version: Block::VERSION,
            validators: ValidatorSlots::default(),
            block_number,
            view_number,
            parent_macro_hash,
            seed: seed.clone(),
            parent_hash,
            state_root: Blake2bHash::default(),
            extrinsics_root: Blake2bHash::default(),
            timestamp,
            transactions_root: Blake2bHash::default(),
        };

        let state = self.blockchain.state();
        state
            .reward_registry()
            .commit_block(
                txn,
                &Block::Macro(MacroBlock {
                    header: header.clone(),
                    justification: None,
                    extrinsics: None,
                }),
                self.blockchain.view_number(),
            )
            .expect("Failed to commit dummy block to reward registry");

        let mut inherents = self
            .blockchain
            .finalize_last_epoch(&self.blockchain.state(), &header);

        // Add slashes for view changes.
        let view_changes = ViewChanges::new(
            header.block_number,
            self.blockchain.view_number(),
            header.view_number,
        );
        inherents.append(&mut self.blockchain.create_slash_inherents(
            &[],
            &view_changes,
            Some(txn),
        ));

        // Rewards are distributed with delay.
        state
            .accounts()
            .commit(txn, &[], &inherents, block_number)
            .expect("Failed to compute accounts hash during block production");

        let state_root = state.accounts().hash(Some(txn));

        let transactions_root = self
            .blockchain
            .get_transactions_root(policy::epoch_at(block_number), Some(txn))
            .expect("Failed to compute transactions root, micro blocks missing");

        let validators = self.blockchain.next_validators(seed, Some(txn));

        header.validators = validators.into();
        header.state_root = state_root;
        header.transactions_root = transactions_root;

        header
    }

    fn next_micro_header(
        &self,
        timestamp: u64,
        view_number: u32,
        extrinsics: &MicroExtrinsics,
        view_changes: &Option<ViewChanges>,
    ) -> MicroHeader {
        let block_number = self.blockchain.height() + 1;
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp() + 1);

        let parent_hash = self.blockchain.head_hash();
        let extrinsics_root = extrinsics.hash();

        let inherents =
            self.blockchain
                .create_slash_inherents(&extrinsics.fork_proofs, view_changes, None);
        // Rewards are distributed with delay.
        let state_root = self
            .blockchain
            .state()
            .accounts()
            .hash_with(&extrinsics.transactions, &inherents, block_number)
            .expect("Failed to compute accounts hash during block production");

        let seed = self
            .blockchain
            .head()
            .seed()
            .sign_next(&self.validator_key.secret);

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
