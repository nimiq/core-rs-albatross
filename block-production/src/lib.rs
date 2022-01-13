use nimiq_account::Inherent;
use nimiq_block::{
    ForkProof, MacroBlock, MacroBody, MacroHeader, MicroBlock, MicroBody, MicroHeader,
    MicroJustification, ViewChangeProof, ViewChanges,
};
use nimiq_blockchain::{AbstractBlockchain, Blockchain, ExtendedTransaction};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::KeyPair as SchnorrKeyPair;
use nimiq_primitives::policy;
use nimiq_transaction::Transaction;

/// Struct that contains all necessary information to actually produce blocks.
/// It has the validator keys for this validator.
#[derive(Clone)]
pub struct BlockProducer {
    pub signing_key: SchnorrKeyPair,
    pub voting_key: BlsKeyPair,
}

impl BlockProducer {
    /// Creates a new BlockProducer struct given a blockchain and a validator key.
    pub fn new(signing_key: SchnorrKeyPair, voting_key: BlsKeyPair) -> Self {
        BlockProducer {
            signing_key,
            voting_key,
        }
    }

    /// Creates the next micro block.
    pub fn next_micro_block(
        &self,
        // The (upgradable) read locked guard to the blockchain
        blockchain: &Blockchain,
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
        // The transactions to be included in the block body.
        mut transactions: Vec<Transaction>,
        // Extra data for this block. It has no a priori use.
        extra_data: Vec<u8>,
    ) -> MicroBlock {
        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = blockchain.block_number() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, blockchain.head().timestamp());

        // Get the hash of the latest block. It can be any block type.
        let parent_hash = blockchain.head_hash();

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let prev_seed = blockchain.head().seed().clone();
        let seed = prev_seed.sign_next(&self.signing_key);

        // Sort the transactions.
        transactions.sort_unstable();

        // Creates a new ViewChanges struct.
        let view_changes = ViewChanges::new(
            blockchain.block_number() + 1,
            blockchain.next_view_number(),
            view_number,
            prev_seed.entropy(),
        );

        log::debug!("Creating inherents");

        // Create the inherents from the fork proofs and the view changes.
        let inherents = blockchain.create_slash_inherents(&fork_proofs, &view_changes, None);

        log::debug!("Updating the state");

        // Update the state and calculate the state root.
        let state_root = blockchain
            .state()
            .accounts
            .get_root_with(&transactions, &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production");

        log::debug!("Calculating the extended transactions");

        // Calculate the extended transactions from the transactions and the inherents.
        let ext_txs = ExtendedTransaction::from(
            blockchain.network_id,
            block_number,
            timestamp,
            transactions.clone(),
            inherents,
        );

        log::debug!("Storing extended transactions into history tree");

        // Store the extended transactions into the history tree and calculate the history root.
        let mut txn = blockchain.write_transaction();

        log::debug!("Computing history root");

        let history_root = blockchain
            .history_store
            .add_to_history(&mut txn, policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.");

        // Not strictly necessary to drop the lock here, but sign as well as compress might be somewhat expensive
        // and there is no need to hold the lock after this point.
        // Abort txn so that blockchain is no longer borrowed.
        txn.abort();

        log::debug!("Creating micro block body");

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

        // Signs the block header using the signing key.
        let hash = header.hash::<Blake2bHash>();
        let signature = self.signing_key.sign(hash.as_slice());

        log::debug!("Returning micro block");

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
        // The (upgradable) read locked guard to the blockchain
        blockchain: &Blockchain,
        // The timestamp for the block proposal.
        timestamp: u64,
        // The view number for the block proposal.
        view_number: u32,
        // Extra data for this block. It has no a priori use.
        extra_data: Vec<u8>,
    ) -> MacroBlock {
        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = blockchain.block_number() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, blockchain.head().timestamp());

        // Get the hash of the latest block (it is by definition a micro block).
        let parent_hash = blockchain.head_hash();

        // Get the hash of the latest election macro block.
        let parent_election_hash = blockchain.election_head_hash();

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let seed = blockchain.head().seed().sign_next(&self.signing_key);

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
        let state = blockchain.state();

        let inherents: Vec<Inherent> = blockchain.create_macro_block_inherents(state, &header);

        // Update the state and add the state root to the header.
        header.state_root = state
            .accounts
            .get_root_with(&[], &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production.");

        // Calculate the extended transactions from the transactions and the inherents.
        let ext_txs = ExtendedTransaction::from(
            blockchain.network_id,
            block_number,
            timestamp,
            vec![],
            inherents,
        );

        // Store the extended transactions into the history tree and calculate the history root.
        let mut txn = blockchain.write_transaction();

        header.history_root = blockchain
            .history_store
            .add_to_history(&mut txn, policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.");

        txn.abort();

        // Calculate the disabled set for the current validator set.
        // Note: We are fetching the previous disabled set here because we have already updated the
        // state. So the staking contract has already moved the disabled set for this batch into the
        // previous disabled set.
        let disabled_set = blockchain.get_staking_contract().previous_disabled_slots();

        // Calculate the lost reward set for the current validator set.
        // Note: We are fetching the previous lost rewards set here because we have already updated the
        // state. So the staking contract has already moved the lost rewards set for this batch into the
        // previous lost rewards set.
        let lost_reward_set = blockchain.get_staking_contract().previous_lost_rewards();

        // If this is an election block, calculate the validator set for the next epoch.
        let validators = if policy::is_election_block_at(blockchain.block_number() + 1) {
            Some(blockchain.next_validators(&header.seed))
        } else {
            None
        };

        // Calculate the pk_tree_root.
        let pk_tree_root = validators.as_ref().map(MacroBlock::pk_tree_root);

        // Create the body for the macro block.
        let body = MacroBody {
            validators,
            pk_tree_root,
            lost_reward_set,
            disabled_set,
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
