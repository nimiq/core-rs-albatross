use std::sync::Arc;
use std::sync::RwLock;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, MacroBlock, MacroBody, PbftCommitMessage, PbftPrepareMessage, PbftProofBuilder,
    PbftProposal, SignedPbftCommitMessage, SignedPbftPrepareMessage, SignedViewChange, ViewChange,
    ViewChangeProof, ViewChangeProofBuilder,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::{Blockchain, ForkEvent, PushError, PushResult};
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::Environment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::policy;

mod macro_block_sync;
mod signed;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

struct TemporaryBlockProducer {
    env: Environment,
    blockchain: Arc<Blockchain>,
    producer: BlockProducer,
}

impl TemporaryBlockProducer {
    fn new() -> Self {
        let env = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());

        let keypair = KeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        );
        let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);
        TemporaryBlockProducer {
            env,
            blockchain,
            producer,
        }
    }

    fn push(&self, block: Block) -> Result<PushResult, PushError> {
        self.blockchain.push(block)
    }

    fn next_block(&self, view_number: u32, extra_data: Vec<u8>) -> Block {
        let height = self.blockchain.block_number() + 1;

        let view_change_proof = if self.blockchain.next_view_number() == view_number {
            None
        } else {
            Some(self.create_view_change_proof(view_number))
        };

        let block = if policy::is_macro_block_at(height) {
            let (proposal, extrinsics) = self.producer.next_macro_block_proposal(
                self.blockchain.time.now() + height as u64 * 1000,
                0u32,
                view_change_proof,
                extra_data,
            );
            Block::Macro(TemporaryBlockProducer::finalize_macro_block(
                proposal, extrinsics,
            ))
        } else {
            Block::Micro(self.producer.next_micro_block(
                self.blockchain.time.now() + height as u64 * 1000,
                view_number,
                view_change_proof,
                vec![],
                extra_data,
            ))
        };
        assert_eq!(self.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    fn finalize_macro_block(proposal: PbftProposal, extrinsics: MacroBody) -> MacroBlock {
        let keypair = KeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        );

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
                block_hash: block_hash,
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
            body: Some(extrinsics),
        }
    }

    fn create_view_change_proof(&self, view_number: u32) -> ViewChangeProof {
        let keypair = KeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        );

        let view_change = ViewChange {
            block_number: self.blockchain.block_number() + 1,
            new_view_number: view_number,
            prev_seed: self.blockchain.head().seed().clone(),
        };

        // create signed prepare and commit
        let view_change = SignedViewChange::from_message(view_change, &keypair.secret_key, 0);

        // create proof
        let mut view_change_proof = ViewChangeProofBuilder::new();
        view_change_proof.add_signature(&keypair.public_key, policy::SLOTS, &view_change);
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
    temp_producer2.push(ancestor.clone()).unwrap();

    let mut previous = ancestor;
    for _ in 0..policy::EPOCH_LENGTH {
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
    let _macro_block = loop {
        block = temp_producer1.next_block(0, vec![]);
        temp_producer2.push(block.clone()).unwrap();
        if let Block::Macro(macro_block) = block {
            break macro_block;
        }
    };

    let fork1 = temp_producer1.next_block(0, vec![]);
    let fork2 = temp_producer2.next_block(1, vec![]);

    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Ignored));
}

#[test]
fn create_fork_proof() {
    // Build a fork using two producers.
    let producer1 = TemporaryBlockProducer::new();
    let producer2 = TemporaryBlockProducer::new();

    let event1_rc1 = Arc::new(RwLock::new(false));
    let event1_rc2 = event1_rc1.clone();

    producer1
        .blockchain
        .fork_notifier
        .write()
        .register(move |e: &ForkEvent| match e {
            ForkEvent::Detected(_) => *event1_rc2.write().unwrap() = true,
        });

    // Easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = producer1.next_block(0, vec![]);
    let _next_block = producer1.next_block(0, vec![0x48]);
    producer2.push(block).unwrap();

    let fork = producer2.next_block(0, vec![]);
    producer1.push(fork).unwrap();

    // Verify that the fork proof was generated
    assert_eq!(*event1_rc1.read().unwrap(), true);
}
