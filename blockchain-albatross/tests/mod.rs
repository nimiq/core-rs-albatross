use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{Block, MacroBlock, PbftCommitMessage, PbftPrepareMessage, PbftProofBuilder, PbftProposal, SignedPbftCommitMessage, SignedPbftPrepareMessage, ViewChangeProof, SignedViewChange, ViewChange, ViewChangeProofBuilder, MacroExtrinsics};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::blockchain::{Blockchain, PushResult, PushError};
use nimiq_blockchain_base::AbstractBlockchain;
use nimiq_blockchain_base::Direction;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_primitives::{networks::NetworkId};
use nimiq_primitives::policy;
use nimiq_database::Environment;

mod signed;
mod macro_block_sync;

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &'static str = "49ea68eb6b8afdf4ca4d4c0a0b295c76ca85225293693bc30e755476492b707f";

struct TemporaryBlockProducer {
    env: Environment,
    blockchain: Arc<Blockchain>,
    producer: BlockProducer,
}

impl TemporaryBlockProducer {
    fn new() -> Self {
        let env = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());

        let keypair = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
        let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);
        TemporaryBlockProducer {
            env,
            blockchain,
            producer
        }
    }

    fn push(&self, block: Block) -> Result<PushResult, PushError> {
        self.blockchain.push(block)
    }

    fn next_block(&self, view_number: u32, extra_data: Vec<u8>) -> Block {
        let height = self.blockchain.head_height() + 1;

        let view_change_proof = if self.blockchain.next_view_number() == view_number {
            None
        } else {
            Some(self.create_view_change_proof(view_number))
        };

        let block = if policy::is_macro_block_at(height) {
            let (proposal, extrinsics) = self.producer.next_macro_block_proposal(1565713920000 + height as u64 * 2000, 0u32, view_change_proof);
            Block::Macro(TemporaryBlockProducer::finalize_macro_block(proposal, extrinsics))
        } else {
            Block::Micro(self.producer.next_micro_block(vec![], 1565713920000 + height as u64 * 2000, view_number, extra_data, view_change_proof))
        };
        assert_eq!(self.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    fn finalize_macro_block(proposal: PbftProposal, extrinsics: MacroExtrinsics) -> MacroBlock {
        let keypair = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

        let block_hash = proposal.header.hash::<Blake2bHash>();

        // create signed prepare and commit
        let prepare = SignedPbftPrepareMessage::from_message(
            PbftPrepareMessage { block_hash: block_hash.clone() },
            &keypair.secret,
            0);
        let commit = SignedPbftCommitMessage::from_message(
            PbftCommitMessage { block_hash: block_hash.clone() },
            &keypair.secret,
            0);

        // create proof
        let mut pbft_proof = PbftProofBuilder::new();
        pbft_proof.add_prepare_signature(&keypair.public, policy::SLOTS, &prepare);
        pbft_proof.add_commit_signature(&keypair.public, policy::SLOTS, &commit);

        MacroBlock {
            header: proposal.header,
            justification: Some(pbft_proof.build()),
            extrinsics: Some(extrinsics),
        }
    }

    fn create_view_change_proof(&self, view_number: u32) -> ViewChangeProof {
        let keypair = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

        let view_change = ViewChange {
            block_number: self.blockchain.head_height() + 1,
            new_view_number: view_number,
            prev_seed: self.blockchain.head().seed().clone(),
        };

        // create signed prepare and commit
        let view_change = SignedViewChange::from_message(
            view_change,
            &keypair.secret,
            0);

        // create proof
        let mut view_change_proof = ViewChangeProofBuilder::new();
        view_change_proof.add_signature(&keypair.public, policy::SLOTS, &view_change);
        view_change_proof.build()
    }
}

#[test]
fn it_can_rebranch_view_changes() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 1: easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [1] - [1]
    let block = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(0, vec![]);
    let fork1 = temp_producer2.next_block(1, vec![]);

    let inferior2 = temp_producer1.next_block(0, vec![]);
    let fork2 = temp_producer2.next_block(1, vec![]);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Err(PushError::Orphan));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));

    // Case 2: not obvious rebranch rebranch
    // ... - [1] - [1] - [4]
    //          \- [2] - [2]
    let block = temp_producer1.next_block(1, vec![]);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(1, vec![]);
    let fork1 = temp_producer2.next_block(2, vec![]);

    let inferior2 = temp_producer1.next_block(4, vec![]);
    let fork2 = temp_producer2.next_block(2, vec![]);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Err(PushError::Orphan));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));
}

#[test]
fn it_can_rebranch_forks() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 1: easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(block).unwrap();

    let fork1 = temp_producer1.next_block(0, vec![0x48]);
    let fork2 = temp_producer2.next_block(0, vec![]);

    let better = temp_producer1.next_block(0, vec![]);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Forked));

    // Check that producer 2 rebranches.
    assert_eq!(temp_producer2.push(better), Ok(PushResult::Rebranched));

    // Case 2: more difficult rebranch
    //              a     b     c     d
    // [0] - [0] - [0] - [0] - [0] - [0]
    //          \- [0] - [0] - [1] - [1]
    let block = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(block).unwrap();

    let fork1a = temp_producer1.next_block(0, vec![0x48]);
    let fork2a = temp_producer2.next_block(0, vec![]);

    let fork1b = temp_producer1.next_block(0, vec![]);
    let fork2b = temp_producer2.next_block(0, vec![]);

    let fork1c = temp_producer1.next_block(0, vec![]);
    let fork2c = temp_producer2.next_block(1, vec![]);

    let fork1d = temp_producer1.next_block(0, vec![]);
    let fork2d = temp_producer2.next_block(1, vec![]);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2a), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1a), Ok(PushResult::Forked));
    assert_eq!(temp_producer1.push(fork2b), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1b), Ok(PushResult::Forked));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork2c), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1c), Ok(PushResult::Ignored));

    assert_eq!(temp_producer1.push(fork2d), Ok(PushResult::Extended));
    assert_eq!(temp_producer2.push(fork1d), Err(PushError::Orphan));
}

#[test]
fn it_cant_rebranch_across_epochs() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // The number in [_] represents the epoch number
    //              a
    // [0] - [0] - [0]
    //          \- [0] - ... - [0] - [1]

    let ancestor = temp_producer1.next_block(0, vec![]);
    temp_producer2.push(ancestor.clone());

    let mut previous = ancestor;
    for _ in 0 .. policy::EPOCH_LENGTH {
        previous = temp_producer1.next_block(0, vec![]);
    }

    let fork = temp_producer2.next_block(1, vec![]);
    assert_eq!(temp_producer1.push(fork), Err(PushError::InvalidFork));
}

#[test]
fn it_can_rebranch_at_macro_block() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // The numbers in [X/Y] represent block_number (X) and view_number (Y):
    //
    // [0/0] ... [1/0] - [1/0]
    //                \- [1/1]

    let mut block;
    let macro_block = loop {
        block = temp_producer1.next_block(0, vec![]);
        temp_producer2.push(block.clone());
        if let Block::Macro(macro_block) = block {
            break macro_block;
        }
    };

    let fork1 = temp_producer1.next_block(0, vec![]);
    let fork2 = temp_producer2.next_block(1, vec![]);

    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Ignored));
}
