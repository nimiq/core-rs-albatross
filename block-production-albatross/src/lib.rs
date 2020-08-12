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

use std::sync::Arc;

use block::ForkProof;
use block::MicroJustification;
use block::{
    Block, MacroBlock, MacroBody, MacroHeader, MicroBlock, MicroBody, MicroHeader, PbftProposal,
    ViewChangeProof, ViewChanges,
};
use blockchain::blockchain::Blockchain;
use blockchain::chain_info::ChainInfo;
use blockchain::slots::ForkProofInfos;
use blockchain_base::AbstractBlockchain;
use bls::KeyPair;
use database::WriteTransaction;
use hash::{Blake2bHash, Hash};
use mempool::Mempool;
use nimiq_account::Inherent;
use primitives::policy;

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
        _extra_data: Vec<u8>,
    ) -> (PbftProposal, MacroBody) {
        // Creates the extrinsics for the block proposal.
        let extrinsics = self.next_macro_extrinsics();

        // Creates the header for the block proposal.
        let mut txn = self.blockchain.write_transaction();
        let header = self.next_macro_header(&mut txn, timestamp, view_number, &extrinsics);
        txn.abort();

        // Returns the block proposal.
        (
            PbftProposal {
                header,
                view_change: view_change_proof,
            },
            extrinsics,
        )
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
        // Creates a new ViewChanges struct.
        let view_changes = ViewChanges::new(
            self.blockchain.block_number() + 1,
            self.blockchain.next_view_number(),
            view_number,
        );

        // Creates the extrinsics for the block.
        let extrinsics = self.next_micro_extrinsics(fork_proofs, &view_changes, extra_data);

        // Creates the header for the block.
        let header = self.next_micro_header(timestamp, view_number, &extrinsics, &view_changes);

        // Signs the block header using the validator key.
        let signature = self.validator_key.sign(&header).compress();

        // Returns the micro block.
        MicroBlock {
            header,
            body: Some(extrinsics),
            justification: Some(MicroJustification {
                signature,
                view_change_proof,
            }),
        }
    }

    /// Creates the extrinsics for the next macro block.
    pub fn next_macro_extrinsics(&self) -> MacroBody {
        let slashed_set = self
            .blockchain
            .slashed_set_at(self.blockchain.block_number() + 1)
            .expect("Missing previous block for block production")
            .next_slashed_set(self.blockchain.block_number() + 1);

        // TODO: Compute the history root and the validator list?
        MacroBody::from_slashed_set(
            slashed_set.prev_epoch_state.clone(),
            slashed_set.current_epoch(),
        )
    }

    /// Creates the extrinsics for the next micro block.
    fn next_micro_extrinsics(
        &self,
        fork_proofs: Vec<ForkProof>,
        view_changes: &Option<ViewChanges>,
        extra_data: Vec<u8>,
    ) -> MicroBody {
        // Calculate the maximum allowed size for the micro block extrinsics.
        let max_size = MicroBlock::MAX_SIZE
            - MicroHeader::SIZE
            - MicroBody::get_metadata_size(fork_proofs.len());

        // Get the transactions from the mempool.
        let mut transactions = self
            .mempool
            .as_ref()
            .map(|mempool| mempool.get_transactions_for_block(max_size))
            .unwrap_or_else(Vec::new);

        // Create the inherents from the fork proofs and the view changes.
        let inherents = self
            .blockchain
            .create_slash_inherents(&fork_proofs, view_changes, None);

        // Collect the receipts generated from all the transactions and inherents.
        self.blockchain
            .state()
            .accounts()
            .collect_receipts(
                &transactions,
                &inherents,
                self.blockchain.block_number() + 1,
            )
            .expect("Failed to collect receipts during block production");

        // Sort the transactions.
        transactions.sort_unstable_by(|a, b| a.cmp_block_order(b));

        // Create and return the micro block extrinsics.
        MicroBody {
            fork_proofs,
            transactions,
        }
    }

    /// Creates the header for the next macro block (checkpoint or election).
    pub fn next_macro_header(
        &self,
        txn: &mut WriteTransaction,
        timestamp: u64,
        view_number: u32,
        extrinsics: &MacroBody,
    ) -> MacroHeader {
        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = self.blockchain.block_number() + 1;

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
            block_number,
            view_number,
            parent_election_hash,
            seed: seed.clone(),
            parent_hash,
            state_root: Blake2bHash::default(),
            body_root: extrinsics_root,
            timestamp,
            // TODO: needs to have some data!
            extra_data: vec![],
        };

        // Get and update the state.
        let state = self.blockchain.state();

        // Initialize the inherents vector.
        let mut inherents: Vec<Inherent> = vec![];

        // Calculate the new validators, if this is an election macro block.
        if policy::is_election_block_at(block_number) {
            // Get the new validator list.
            let validators = self.blockchain.next_validators(&seed, Some(txn));

            // TODO: This needs to be moved to the body!!!
            // // Add it to the header.
            // header.validators = validators.into();

            let dummy_macro_block = Block::Macro(MacroBlock {
                header: header.clone(),
                justification: None,
                body: None,
            });
            let prev_chain_info = state.main_chain();
            let chain_info = ChainInfo::new(dummy_macro_block, prev_chain_info).unwrap();
            // For election blocks add reward and finalize epoch inherents.
            inherents.append(&mut self.blockchain.finalize_previous_batch(&state, &chain_info));
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

        // TODO: This needs to be moved to the body!!!
        // // Calculate the transactions root and add it to the header.
        // let transactions_root = self
        //     .blockchain
        //     .get_history_root(policy::epoch_at(block_number), Some(txn))
        //     .expect("Failed to compute transactions root, micro blocks missing");
        // header.history_root = transactions_root;

        // Return the finalized header.
        header
    }

    /// Creates the header for the next micro block.
    fn next_micro_header(
        &self,
        timestamp: u64,
        view_number: u32,
        extrinsics: &MicroBody,
        view_changes: &Option<ViewChanges>,
    ) -> MicroHeader {
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

        // Calculate the root of the extrinsics.
        let extrinsics_root = extrinsics.hash();

        // Create the slash inherents for the view changes and the forks.
        let inherents =
            self.blockchain
                .create_slash_inherents(&extrinsics.fork_proofs, view_changes, None);

        // Calculate the state root.
        let state_root = self
            .blockchain
            .state()
            .accounts()
            .hash_with(&extrinsics.transactions, &inherents, block_number)
            .expect("Failed to compute accounts hash during block production");

        // Create and return the micro block header.
        MicroHeader {
            version: Block::VERSION,
            block_number,
            view_number,
            parent_hash,
            body_root: extrinsics_root,
            state_root,
            seed,
            timestamp,
            // TODO: Add some data!
            extra_data: vec![],
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils {
    use super::*;
    use beserial::Deserialize;
    use block::{
        PbftCommitMessage, PbftPrepareMessage, PbftProofBuilder, SignedPbftCommitMessage,
        SignedPbftPrepareMessage, SignedViewChange, ViewChange, ViewChangeProofBuilder,
    };
    use blockchain_base::PushResult;
    use bls::lazy::LazyPublicKey;
    use keys::Address;
    use nimiq_vrf::VrfSeed;
    use primitives::slot::{ValidatorSlotBand, ValidatorSlots};

    // Fill epoch with micro blocks
    pub fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
        let init_height = blockchain.head_height();
        let macro_block_number = policy::macro_block_after(init_height + 1);
        for i in (init_height + 1)..macro_block_number {
            let last_micro_block = producer.next_micro_block(
                blockchain.time.now() + i as u64 * 1000,
                0,
                None,
                vec![],
                vec![0x42],
            );
            assert_eq!(
                blockchain.push(Block::Micro(last_micro_block)),
                Ok(PushResult::Extended)
            );
        }
        assert_eq!(blockchain.head_height(), macro_block_number - 1);
    }

    pub fn sign_macro_block(
        keypair: &KeyPair,
        proposal: PbftProposal,
        extrinsics: Option<MacroBody>,
    ) -> MacroBlock {
        let block_hash = proposal.header.hash::<Blake2bHash>();

        // create signed prepare and commit
        let prepare = SignedPbftPrepareMessage::from_message(
            PbftPrepareMessage {
                block_hash: block_hash.clone(),
            },
            &keypair.secret_key,
            0,
        );
        let commit = SignedPbftCommitMessage::from_message(
            PbftCommitMessage {
                block_hash: block_hash.clone(),
            },
            &keypair.secret_key,
            0,
        );

        // create proof
        let mut pbft_proof = PbftProofBuilder::new();
        pbft_proof.add_prepare_signature(&keypair.public_key, policy::SLOTS, &prepare);
        pbft_proof.add_commit_signature(&keypair.public_key, policy::SLOTS, &commit);

        MacroBlock {
            header: proposal.header,
            justification: Some(pbft_proof.build()),
            body: extrinsics,
        }
    }

    pub fn sign_view_change(
        keypair: &KeyPair,
        prev_seed: VrfSeed,
        block_number: u32,
        new_view_number: u32,
    ) -> ViewChangeProof {
        let view_change = ViewChange {
            block_number,
            new_view_number,
            prev_seed,
        };
        let signed_view_change =
            SignedViewChange::from_message(view_change.clone(), &keypair.secret_key, 0);

        let mut proof_builder = ViewChangeProofBuilder::new();
        proof_builder.add_signature(&keypair.public_key, policy::SLOTS, &signed_view_change);
        assert_eq!(
            proof_builder.verify(&view_change, policy::TWO_THIRD_SLOTS),
            Ok(())
        );

        let proof = proof_builder.build();
        let validators = ValidatorSlots::new(vec![ValidatorSlotBand::new(
            LazyPublicKey::from(keypair.public_key),
            Address::default(),
            policy::SLOTS,
        )]);
        assert_eq!(
            proof.verify(&view_change, &validators, policy::TWO_THIRD_SLOTS),
            Ok(())
        );

        proof
    }

    pub fn produce_macro_blocks(
        num_macro: usize,
        producer: &BlockProducer,
        blockchain: &Arc<Blockchain>,
    ) {
        for _ in 0..num_macro {
            fill_micro_blocks(producer, blockchain);

            let next_block_height = blockchain.head_height() + 1;
            let (proposal, extrinsics) = producer.next_macro_block_proposal(
                blockchain.time.now() + blockchain.block_number() as u64 * 1000,
                0u32,
                None,
                vec![],
            );

            let block = sign_macro_block(&producer.validator_key, proposal, Some(extrinsics));
            assert_eq!(
                blockchain.push(Block::Macro(block)),
                Ok(PushResult::Extended)
            );
        }
    }
}
