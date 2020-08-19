use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, BlockError, ForkProof, MacroBlock, MacroBody, PbftCommitMessage, PbftPrepareMessage,
    PbftProofBuilder, PbftProposal, SignedPbftCommitMessage, SignedPbftPrepareMessage,
    SignedViewChange, ViewChange, ViewChangeProof, ViewChangeProofBuilder,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::{Blockchain, PushResult};
use nimiq_blockchain_base::{AbstractBlockchain, PushError};
use nimiq_bls::lazy::LazyPublicKey;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_primitives::policy;
use nimiq_primitives::slot::{ValidatorSlotBand, ValidatorSlots};
use nimiq_vrf::VrfSeed;

use nimiq_block_production_albatross::test_utils::*;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &'static str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

//#[test]
fn it_can_produce_micro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair.clone());

    // #1.0: Empty standard micro block
    let block = producer.next_micro_block(blockchain.time.now(), 0, None, vec![], vec![0x41]);
    assert_eq!(
        blockchain.push(Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.block_number(), 1);

    // Create fork at #1.0
    let fork_proof: ForkProof;
    {
        let header1 = block.header.clone();
        let justification1 = block.justification.unwrap().signature.clone();
        let mut header2 = header1.clone();
        header2.timestamp += 1;
        let justification2 = keypair.sign(&header2).compress();
        fork_proof = ForkProof {
            header1,
            header2,
            justification1,
            justification2,
        };
    }

    // #2.0: Empty micro block with fork proof
    let block = producer.next_micro_block(
        blockchain.time.now() + 1000,
        0,
        None,
        vec![fork_proof],
        vec![0x41],
    );
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.block_number(), 2);

    // #2.1: Empty view-changed micro block (wrong prev_hash)
    let view_change = sign_view_change(VrfSeed::default(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Err(PushError::InvalidBlock(BlockError::InvalidJustification))
    );

    // #2.2: Empty view-changed micro block
    let view_change = sign_view_change(blockchain.head().seed().clone(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.block_number(), 3);
    assert_eq!(blockchain.next_view_number(), 1);
}

// Fill epoch with micro blocks
fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
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

fn sign_macro_block(proposal: PbftProposal, extrinsics: Option<MacroBody>) -> MacroBlock {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

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

fn sign_view_change(
    prev_seed: VrfSeed,
    block_number: u32,
    new_view_number: u32,
) -> ViewChangeProof {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

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

//#[test]
fn it_can_produce_macro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair.clone());

    fill_micro_blocks(&producer, &blockchain);

    let (proposal, extrinsics) = producer.next_macro_block_proposal(
        blockchain.time.now() + blockchain.block_number() as u64 * 1000,
        0u32,
        None,
        vec![],
    );

    let block = sign_macro_block(proposal, Some(extrinsics));
    assert_eq!(
        blockchain.push(Block::Macro(block)),
        Ok(PushResult::Extended)
    );
}

//#[test]
fn it_can_produce_election_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair.clone());
    // push micro and macro blocks until the 3rd epoch is reached
    while policy::epoch_at(blockchain.block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain);

        let (proposal, extrinsics) = producer.next_macro_block_proposal(
            blockchain.time.now() + blockchain.block_number() as u64 * 1000,
            0u32,
            None,
            vec![0x42],
        );

        let block = sign_macro_block(proposal, Some(extrinsics));
        assert_eq!(
            blockchain.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

// TODO Test transactions
