use std::sync::Arc;

use nimiq_account::Inherent;
use nimiq_block::{
    ForkProof, MacroBlock, MacroBody, MacroHeader, MicroBlock, MicroBody, MicroHeader,
    MicroJustification, ViewChangeProof, ViewChanges,
};
use nimiq_blockchain::{AbstractBlockchain, Blockchain, ExtendedTransaction};
use nimiq_bls::KeyPair;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::Mempool;
use nimiq_primitives::policy;

/// Struct that contains all necessary information to actually produce blocks. It has the current
/// blockchain store and state, the current mempool for this validator and the validator key for
/// this validator.
pub struct BlockProducer {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Option<Arc<Mempool>>,
    pub validator_key: KeyPair,
}

impl BlockProducer {
    /// Creates a new BlockProducer struct given a blockchain, a mempool and a validator key.
    pub fn new(blockchain: Arc<Blockchain>, mempool: Arc<Mempool>, validator_key: KeyPair) -> Self {
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

    /// Creates the next micro block. By definition it is already finalized.
    // Note: Needs to be called with the Blockchain lock held.
    pub fn next_micro_block(
        &self,
        // The timestamp for the block.
        timestamp: u64,
        // The view number for the block.
        view_number: u32,
        // The view change proof. Only exists if one or more view changes happened for this block
        // height.
        view_change_proof: Option<ViewChangeProof>,
        // Proofs of any forks created by malicious validators. A fork proof may be submitted during
        // the batch when it happened or in the next one, but not after that.
        fork_proofs: Vec<ForkProof>,
        // Extra data for this block. It has no a priori use.
        extra_data: Vec<u8>,
    ) -> MicroBlock {
        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = self.blockchain.block_number() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp());

        // Get the hash of the latest block. It can be any block type.
        let parent_hash = self.blockchain.head_hash();

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let seed = self
            .blockchain
            .head()
            .seed()
            .sign_next(&self.validator_key.secret_key);

        // Calculate the maximum allowed size for the micro block body.
        let max_size = MicroBlock::MAX_SIZE
            - MicroHeader::SIZE
            - MicroBody::get_metadata_size(fork_proofs.len());

        // Get the transactions from the mempool.
        let mut transactions = self
            .mempool
            .as_ref()
            .map(|mempool| mempool.get_transactions_for_block(max_size))
            .unwrap_or_else(Vec::new);

        // Sort the transactions.
        transactions.sort_unstable_by(|a, b| a.cmp_block_order(b));

        // Creates a new ViewChanges struct.
        let view_changes = ViewChanges::new(
            self.blockchain.block_number() + 1,
            self.blockchain.next_view_number(),
            view_number,
        );

        // Create the inherents from the fork proofs and the view changes.
        let inherents = self
            .blockchain
            .create_slash_inherents(&fork_proofs, &view_changes, None);

        // Update the state and calculate the state root.
        let state_root = self
            .blockchain
            .state()
            .accounts
            .hash_with(&transactions, &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production");

        // Calculate the extended transactions from the transactions and the inherents.
        let ext_txs =
            ExtendedTransaction::from(block_number, timestamp, transactions.clone(), inherents);

        // Store the extended transactions into the history tree and calculate the history root.
        let mut txn = self.blockchain.write_transaction();

        let history_root = self
            .blockchain
            .history_store
            .add_to_history(&mut txn, policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.");

        // Create the micro block body.
        let body = MicroBody {
            fork_proofs,
            transactions,
        };

        // Create the micro block header.
        let header = MicroHeader {
            version: policy::VERSION,
            block_number,
            view_number,
            timestamp,
            parent_hash,
            seed,
            extra_data,
            state_root,
            body_root: body.hash(),
            history_root,
        };

        // Signs the block header using the validator key.
        let signature = self.validator_key.sign(&header).compress();

        // Returns the micro block.
        MicroBlock {
            header,
            body: Some(body),
            justification: Some(MicroJustification {
                signature,
                view_change_proof,
            }),
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
        // Extra data for this block. It has no a priori use.
        extra_data: Vec<u8>,
    ) -> MacroBlock {
        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = self.blockchain.block_number() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, self.blockchain.head().timestamp());

        // Get the hash of the latest block (it is by definition a micro block).
        let parent_hash = self.blockchain.head_hash();

        // Get the hash of the latest election macro block.
        let parent_election_hash = self.blockchain.election_head_hash();

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let seed = self
            .blockchain
            .head()
            .seed()
            .sign_next(&self.validator_key.secret_key);

        // Create the header for the macro block without the state root and the transactions root.
        // We need several fields of this header in order to calculate the transactions and the
        // state.
        let mut header = MacroHeader {
            version: policy::VERSION,
            block_number,
            view_number,
            timestamp,
            parent_hash,
            parent_election_hash,
            seed,
            extra_data,
            state_root: Blake2bHash::default(),
            body_root: Blake2bHash::default(),
            history_root: Blake2bHash::default(),
        };

        // Get the state.
        let state = self.blockchain.state();

        let inherents: Vec<Inherent> = self
            .blockchain
            .create_macro_block_inherents(&state, &header);

        let transactions = self.blockchain.create_txs_from_inherents(&inherents);

        // Update the state and add the state root to the header.
        header.state_root = state
            .accounts
            .hash_with(&[], &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production.");

        // Calculate the extended transactions from the transactions and the inherents.
        let ext_txs =
            ExtendedTransaction::from(block_number, timestamp, transactions.clone(), inherents);

        // Store the extended transactions into the history tree and calculate the history root.
        let mut txn = self.blockchain.write_transaction();

        header.history_root = self
            .blockchain
            .history_store
            .add_to_history(&mut txn, policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.");

        // Calculate the disabled set for the current validator set.
        // Note: We are fetching the previous disabled set here because we have already updated the
        // state. So the staking contract has already moved the disabled set for this batch into the
        // previous disabled set.
        let disabled_set = self
            .blockchain
            .get_staking_contract()
            .previous_disabled_slots();

        // Calculate the lost reward set for the current validator set.
        // Note: We are fetching the previous lost rewards set here because we have already updated the
        // state. So the staking contract has already moved the lost rewards set for this batch into the
        // previous lost rewards set.
        let lost_reward_set = self
            .blockchain
            .get_staking_contract()
            .previous_lost_rewards();

        // If this is an election block, calculate the validator set for the next epoch.
        let validators = if policy::is_election_block_at(self.blockchain.block_number() + 1) {
            Some(self.blockchain.next_validators(&header.seed))
        } else {
            None
        };

        // Create the body for the macro block.
        let body = MacroBody {
            validators,
            lost_reward_set,
            disabled_set,
            transactions,
        };

        // Add the root of the body to the header.
        header.body_root = body.hash();

        // Returns the block proposal.
        MacroBlock {
            header,
            body: Some(body),
            justification: None,
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
