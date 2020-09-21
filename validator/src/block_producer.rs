extern crate nimiq_block_albatross as block;
extern crate nimiq_blockchain_albatross as blockchain;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_database as database;
extern crate nimiq_genesis as genesis;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_mempool as mempool;
extern crate nimiq_primitives as primitives;
extern crate nimiq_vrf as vrf;

use std::sync::Arc;

use block::ForkProof;
use block::MicroJustification;
use block::{
    Block, MacroBlock, MacroExtrinsics, MacroHeader, MicroBlock, MicroExtrinsics, MicroHeader,
    PbftProposal, ViewChangeProof, ViewChanges,
};
use blockchain::blockchain::Blockchain;
use bls::KeyPair;
use database::WriteTransaction;
use hash::{Blake2bHash, Hash};
use mempool::Mempool;
use nimiq_account::Inherent;
use primitives::policy;
use crate::tendermint_outside_deps::Proposal;
use nimiq_blockchain_albatross::slots::ForkProofInfos;
use nimiq_blockchain_albatross::chain_info::ChainInfo;

/// Struct that contains all necessary information to actually produce blocks. It has the current
/// blockchain store and state, the current mempool for this validator and the validator key for
/// this validator.
pub struct BlockProducer {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Option<Arc<Mempool<Blockchain>>>,
    pub validator_key: KeyPair,
}

impl BlockProducer {
    /// Creates a new BlockProducer struct given a blockchain, a mempool and a validator key.
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

    /// Creates a new BlockProducer struct without a mempool given a blockchain and a validator key.
    pub fn new_without_mempool(blockchain: Arc<Blockchain>, validator_key: KeyPair) -> Self {
        BlockProducer {
            blockchain,
            mempool: None,
            validator_key,
        }
    }

    /// Creates a proposal for the next macro block (checkpoint or election). It is just a proposal,
    /// NOT a complete block. It still needs to go through the Tendermint protocol in order to be
    /// finalized.
    // Note: Needs to be called with the Blockchain lock held.
    pub fn next_macro_block_proposal(
        &self,
        // The timestamp for the block proposal.
        timestamp: u64,
        // The view number for the block proposal.
        view_number: u32,
        // The view change proof. Only exists if one or more view changes happened for this block
        // height.
        view_change_proof: Option<ViewChangeProof>,
        // Extra data for this block. It has no a priori use.
        extra_data: Vec<u8>,
    ) -> (Proposal, MacroExtrinsics) {
        // Creates the extrinsics for the block proposal.
        let extrinsics = self.next_macro_extrinsics(extra_data);

        // Creates the header for the block proposal.
        let mut txn = self.blockchain.write_transaction();
        let header = self.next_macro_header(&mut txn, timestamp, view_number, &extrinsics);
        txn.abort();

        // Returns the block proposal.
        (
            Proposal {
                header,
            },
            extrinsics,
        )
    }

    /// Creates the extrinsics for the next macro block.
    pub fn next_macro_extrinsics(&self, extra_data: Vec<u8>) -> MacroExtrinsics {
        let slashed_set = self
            .blockchain
            .slashed_set_at(self.blockchain.height() + 1)
            .expect("Missing previous block for block production")
            .next_slashed_set(self.blockchain.height() + 1);

        MacroExtrinsics {
            slashed_set: slashed_set.prev_epoch_state.clone(),
            current_slashed_set: slashed_set.current_epoch(),
            extra_data,
        }
    }

    /// Creates the header for the next macro block (checkpoint or election).
    pub fn next_macro_header(
        &self,
        txn: &mut WriteTransaction,
        timestamp: u64,
        view_number: u32,
        extrinsics: &MacroExtrinsics,
    ) -> MacroHeader {
        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = self.blockchain.height() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp());

        // Get the hash of the latest block (it is by definition a micro block).
        let parent_hash = self.blockchain.head_hash();

        // Get the hash of the latest macro block (it is by definition a checkpoint macro block).
        let parent_macro_hash = self.blockchain.macro_head_hash();

        // Get the hash of the latest election macro block.
        let parent_election_hash = self.blockchain.election_head_hash();

        // Calculate the root of the extrinsics.
        let extrinsics_root = extrinsics.hash();

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let seed = self
            .blockchain
            .head()
            .seed()
            .sign_next(&self.validator_key.secret_key);

        // Create the header for the macro block without the state root and the transactions root.
        // We need several fields of this header in order to calculate the transactions and the
        // state. It is just simpler to pass the entire header.
        let mut header = MacroHeader {
            version: Block::VERSION,
            // The default is to have no new validators. If this block turns out to be an election
            // block the validators will be set later in this function.
            validators: None,
            block_number,
            view_number,
            parent_macro_hash,
            parent_election_hash,
            seed: seed.clone(),
            parent_hash,
            state_root: Blake2bHash::default(),
            extrinsics_root,
            timestamp,
            transactions_root: Blake2bHash::default(),
        };

        // Get and update the state.
        let state = self.blockchain.state();

        // Initialize the inherents vector.
        let mut inherents: Vec<Inherent> = vec![];

        // Calculate the new validators, if this is an election macro block.
        if policy::is_election_block_at(block_number) {
            // Get the new validator list.
            let validators = self.blockchain.next_validators(&seed, Some(txn));

            // Add it to the header.
            header.validators = validators.into();

            let dummy_macro_block = Block::Macro(MacroBlock {
                header: header.clone(),
                justification: None,
                extrinsics: None,
            });
            let prev_chain_info = state.main_chain();
            let fork_proof_infos = ForkProofInfos::empty();
            let chain_info =
                ChainInfo::new(dummy_macro_block, prev_chain_info, &fork_proof_infos).unwrap();
            // For election blocks add reward and finalize epoch inherents.
            inherents.append(&mut self.blockchain.finalize_previous_epoch(&state, &chain_info));
        }

        // Create the slash inherents for the view changes.
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

        // Update the state again to distribute the rewards.
        state
            .accounts()
            .commit(txn, &[], &inherents, block_number)
            .expect("Failed to compute accounts hash during block production");

        // Calculate the state root and add it to the header.
        let state_root = state.accounts().hash(Some(txn));
        header.state_root = state_root;

        // Calculate the transactions root and add it to the header.
        let transactions_root = self
            .blockchain
            .get_transactions_root(policy::epoch_at(block_number), Some(txn))
            .expect("Failed to compute transactions root, micro blocks missing");
        header.transactions_root = transactions_root;

        // Return the finalized header.
        header
    }
}
