extern crate nimiq_block_albatross as block;
extern crate nimiq_blockchain_albatross as blockchain;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_database as database;
extern crate nimiq_genesis as genesis;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_mempool as mempool;
extern crate nimiq_primitives as primitives;

use std::sync::Arc;

use block::MicroJustification;
use block::{ForkProof, MacroBlock};
use block::{MacroBody, MacroHeader, MicroBlock, MicroBody, MicroHeader, ViewChangeProof, ViewChanges};
use blockchain::blockchain::Blockchain;
use blockchain::history_store::ExtendedTransaction;
use bls::KeyPair;

use hash::{Blake2bHash, Hash};
use mempool::Mempool;
use nimiq_account::Inherent;
use primitives::policy;

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
        let seed = self.blockchain.head().seed().sign_next(&self.validator_key.secret_key);

        // Calculate the maximum allowed size for the micro block body.
        let max_size = MicroBlock::MAX_SIZE - MicroHeader::SIZE - MicroBody::get_metadata_size(fork_proofs.len());

        // Get the transactions from the mempool.
        let mut transactions = self
            .mempool
            .as_ref()
            .map(|mempool| mempool.get_transactions_for_block(max_size))
            .unwrap_or_else(Vec::new);

        // Sort the transactions.
        transactions.sort_unstable_by(|a, b| a.cmp_block_order(b));

        // Creates a new ViewChanges struct.
        let view_changes = ViewChanges::new(self.blockchain.block_number() + 1, self.blockchain.next_view_number(), view_number);

        // Create the inherents from the fork proofs and the view changes.
        let inherents = self.blockchain.create_slash_inherents(&fork_proofs, &view_changes, None);

        // Update the state and calculate the state root.
        let state_root = self
            .blockchain
            .state()
            .accounts()
            .hash_with(&transactions, &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production");

        // Create the micro block body.
        let body = MicroBody { fork_proofs, transactions };

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
        };

        // Signs the block header using the validator key.
        let signature = self.validator_key.sign(&header).compress();

        // Returns the micro block.
        MicroBlock {
            header,
            body: Some(body),
            justification: Some(MicroJustification { signature, view_change_proof }),
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
        let seed = self.blockchain.head().seed().sign_next(&self.validator_key.secret_key);

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
        };

        // Get the state.
        let state = self.blockchain.state();

        // Initialize the inherents vector.
        let mut inherents: Vec<Inherent> = vec![];

        // All macro blocks are the end of a batch, so finalize the batch.
        inherents.append(&mut self.blockchain.finalize_previous_batch(&state, &header));

        // If this is an election macro block, then it also is the end of an epoch. So finalize the
        // epoch.
        if policy::is_election_block_at(block_number) {
            inherents.push(self.blockchain.finalize_previous_epoch());
        }

        // Update the state and add the state root to the header.
        header.state_root = state
            .accounts()
            .hash_with(&[], &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production.");

        // Calculate the extended transactions from the transactions and the inherents.
        let ext_txs = ExtendedTransaction::from(block_number, timestamp, vec![], inherents);

        // Store the extended transactions into the history tree and calculate the history root.
        let mut txn = self.blockchain.write_transaction();

        let history_root = self
            .blockchain
            .history_store
            .add_to_history(&mut txn, policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.");

        // Calculate the disabled set for the current validator set.
        let disabled_set = self.blockchain.get_staking_contract().previous_disabled_slots();

        // Calculate the lost reward set for the current validator set.
        let lost_reward_set = self.blockchain.get_staking_contract().previous_lost_rewards();

        // If this is an election block, calculate the validator set for the next epoch.
        let validators = if policy::is_election_block_at(self.blockchain.block_number() + 1) {
            Some(self.blockchain.next_slots(&header.seed).validator_slots)
        } else {
            None
        };

        // Create the body for the macro block.
        let body = MacroBody {
            validators,
            lost_reward_set,
            disabled_set,
            history_root,
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
pub mod test_utils {
    use block::{Block, MacroBlock, MultiSignature, TendermintIdentifier, TendermintProof, TendermintStep, TendermintVote};
    use blockchain::PushResult;
    use bls::AggregateSignature;
    use collections::BitSet;
    use nimiq_nano_sync::primitives::pk_tree_construct;
    use primitives::policy::{SLOTS, TWO_THIRD_SLOTS};

    use super::*;

    // Fill epoch with micro blocks
    pub fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
        let init_height = blockchain.block_number();
        let macro_block_number = policy::macro_block_after(init_height + 1);
        for i in (init_height + 1)..macro_block_number {
            let last_micro_block = producer.next_micro_block(blockchain.time.now() + i as u64 * 1000, 0, None, vec![], vec![0x42]);
            assert_eq!(blockchain.push(Block::Micro(last_micro_block)), Ok(PushResult::Extended));
        }
        assert_eq!(blockchain.block_number(), macro_block_number - 1);
    }

    pub fn sign_macro_block(keypair: &KeyPair, header: MacroHeader, body: Option<MacroBody>) -> MacroBlock {
        // Calculate block hash.
        let block_hash = header.hash::<Blake2bHash>();

        // Calculate the validator Merkle root (used in the nano sync).
        let validator_merkle_root = pk_tree_construct(vec![keypair.public_key.public_key; SLOTS as usize]);

        // Create the precommit tendermint vote.
        let precommit = TendermintVote {
            proposal_hash: Some(block_hash),
            id: TendermintIdentifier {
                block_number: header.block_number,
                round_number: 0,
                step: TendermintStep::PreCommit,
            },
            validator_merkle_root,
        };

        // Create signed precommit.
        let signed_precommit = keypair.secret_key.sign(&precommit);

        // Create signers Bitset.
        let mut signers = BitSet::new();
        for i in 0..TWO_THIRD_SLOTS {
            signers.insert(i as usize);
        }

        // Create multisignature.
        let multisig = MultiSignature {
            signature: AggregateSignature::from_signatures(&*vec![signed_precommit; TWO_THIRD_SLOTS as usize]),
            signers,
        };

        // Create Tendermint proof.
        let tendermint_proof = TendermintProof { round: 0, sig: multisig };

        // Create and return the macro block.
        MacroBlock {
            header,
            body,
            justification: Some(tendermint_proof),
        }
    }

    // /// Currently unused
    // pub fn sign_view_change(
    //     keypair: &KeyPair,
    //     prev_seed: VrfSeed,
    //     block_number: u32,
    //     new_view_number: u32,
    // ) -> ViewChangeProof {
    //     // Create the view change.
    //     let view_change = ViewChange {
    //         block_number,
    //         new_view_number,
    //         prev_seed,
    //     };

    //     // Sign the view change.
    //     let signed_view_change =
    //         SignedViewChange::from_message(view_change.clone(), &keypair.secret_key, 0).signature;

    //     // Create signers Bitset.
    //     let mut signers = BitSet::new();
    //     for i in 0..TWO_THIRD_SLOTS {
    //         signers.insert(i as usize);
    //     }

    //     // Create ViewChangeProof and return  it.
    //     ViewChangeProof::new(
    //         AggregateSignature::from_signatures(&*vec![
    //             signed_view_change;
    //             TWO_THIRD_SLOTS as usize
    //         ]),
    //         signers,
    //     )
    // }

    pub fn produce_macro_blocks(num_macro: usize, producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
        for _ in 0..num_macro {
            fill_micro_blocks(producer, blockchain);

            let _next_block_height = blockchain.block_number() + 1;
            let macro_block = producer.next_macro_block_proposal(blockchain.time.now() + blockchain.block_number() as u64 * 1000, 0u32, vec![]);

            let block = sign_macro_block(&producer.validator_key, macro_block.header, macro_block.body);
            assert_eq!(blockchain.push(Block::Macro(block)), Ok(PushResult::Extended));
        }
    }
}
