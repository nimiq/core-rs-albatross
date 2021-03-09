use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, BlockError, ForkProof, MacroBlock, MacroBody, MacroHeader, MultiSignature,
    SignedViewChange, TendermintIdentifier, TendermintProof, TendermintStep, TendermintVote,
    ViewChange, ViewChangeProof,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::{Blockchain, PushError, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair, SecretKey};
use nimiq_collections::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_nano_sync::pk_tree_construct;
use nimiq_primitives::policy;
use nimiq_primitives::policy::{SLOTS, TWO_THIRD_SLOTS};
use nimiq_vrf::VrfSeed;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

// Fill epoch with micro blocks
fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    let init_height = blockchain.block_number();
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
    assert_eq!(blockchain.block_number(), macro_block_number - 1);
}

fn sign_macro_block(header: MacroHeader, body: Option<MacroBody>) -> MacroBlock {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    // Calculate block hash.
    let block_hash = header.hash::<Blake2bHash>();

    // Calculate the validator Merkle root (used in the nano sync).
    let validator_merkle_root =
        pk_tree_construct(vec![keypair.public_key.public_key; SLOTS as usize]);

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
        signature: AggregateSignature::from_signatures(&*vec![
            signed_precommit;
            TWO_THIRD_SLOTS as usize
        ]),
        signers,
    };

    // Create Tendermint proof.
    let tendermint_proof = TendermintProof {
        round: 0,
        sig: multisig,
    };

    // Create and return the macro block.
    MacroBlock {
        header,
        body,
        justification: Some(tendermint_proof),
    }
}

fn sign_view_change(
    prev_seed: VrfSeed,
    block_number: u32,
    new_view_number: u32,
) -> ViewChangeProof {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    // Create the view change.
    let view_change = ViewChange {
        block_number,
        new_view_number,
        prev_seed,
    };

    // Sign the view change.
    let signed_view_change =
        SignedViewChange::from_message(view_change, &keypair.secret_key, 0).signature;

    // Create signers Bitset.
    let mut signers = BitSet::new();
    for i in 0..TWO_THIRD_SLOTS {
        signers.insert(i as usize);
    }

    // Create multisignature.
    let multisig = MultiSignature {
        signature: AggregateSignature::from_signatures(&*vec![
            signed_view_change;
            TWO_THIRD_SLOTS as usize
        ]),
        signers,
    };

    // Create and return view change proof.
    ViewChangeProof { sig: multisig }
}

#[test]
fn it_can_produce_micro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
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
        let justification1 = block.justification.unwrap().signature;
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
    assert_eq!(blockchain.view_number(), 0);

    // #2.1: Empty view-changed micro block (wrong prev_hash)
    let view_change = sign_view_change(VrfSeed::default(), 3, 1);
    let block = producer.next_micro_block(
        blockchain.time.now() + 2000,
        1,
        Some(view_change),
        vec![],
        vec![0x41],
    );

    // the block justification is ok, the view_change justification is not.
    assert_eq!(
        blockchain.push(Block::Micro(block)),
        Err(PushError::InvalidBlock(BlockError::InvalidViewChangeProof))
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

#[test]
fn it_can_produce_macro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    fill_micro_blocks(&producer, &blockchain);

    let macro_block = producer.next_macro_block_proposal(
        blockchain.time.now() + blockchain.block_number() as u64 * 1000,
        0u32,
        vec![],
    );

    let block = sign_macro_block(macro_block.header, macro_block.body);
    assert_eq!(
        blockchain.push(Block::Macro(block)),
        Ok(PushResult::Extended)
    );
}

#[test]
fn it_can_produce_election_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    // push micro and macro blocks until the 3rd epoch is reached
    while policy::epoch_at(blockchain.block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain);

        let macro_block = producer.next_macro_block_proposal(
            blockchain.time.now() + blockchain.block_number() as u64 * 1000,
            0u32,
            vec![0x42],
        );

        let block = sign_macro_block(macro_block.header, macro_block.body);

        assert_eq!(
            blockchain.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

// TODO Test transactions
