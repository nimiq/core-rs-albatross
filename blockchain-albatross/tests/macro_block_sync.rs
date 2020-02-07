use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, MacroBlock, PbftCommitMessage, PbftPrepareMessage, PbftProofBuilder, PbftProposal,
    SignedPbftCommitMessage, SignedPbftPrepareMessage,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::blockchain::{Blockchain, PushResult};
use nimiq_blockchain_base::AbstractBlockchain;
use nimiq_blockchain_base::Direction;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_primitives::networks::NetworkId;
use nimiq_primitives::policy;

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &'static str = "d52d4d68b4e605d5a59c2c0e4f5d4e92ed57ddb65044d85dc6d7b77c819c3610";

// Fill epoch with micro blocks
fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    let init_height = blockchain.head_height();
    let macro_block_number = policy::macro_block_after(init_height + 1);
    for i in (init_height + 1)..macro_block_number {
        let last_micro_block =
            producer.next_micro_block(vec![], 1565713920000 + i as u64 * 2000, 0, vec![0x42], None);
        assert_eq!(
            blockchain.push(Block::Micro(last_micro_block)),
            Ok(PushResult::Extended)
        );
    }
    assert_eq!(blockchain.head_height(), macro_block_number - 1);
}

fn sign_macro_block(proposal: PbftProposal) -> MacroBlock {
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
        extrinsics: None,
    }
}

fn produce_macro_blocks(num_macro: usize, producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    for _ in 0..num_macro {
        fill_micro_blocks(producer, blockchain);

        let next_block_height = blockchain.head_height() + 1;
        let (proposal, _extrinsics) = producer.next_macro_block_proposal(
            1565713920000 + next_block_height as u64 * 2000,
            0u32,
            None,
        );

        let block = sign_macro_block(proposal);
        assert_eq!(
            blockchain.push_block(Block::Macro(block), true),
            Ok(PushResult::Extended)
        );
    }
}

#[test]
fn it_can_sync_macro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());
    let genesis_hash = blockchain.head_hash();

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);

    produce_macro_blocks(2, &producer, &blockchain);

    let macro_blocks = blockchain
        .get_macro_blocks(&genesis_hash, 10, true, Direction::Forward)
        .unwrap();
    assert_eq!(macro_blocks.len(), 2);

    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain2 = Arc::new(Blockchain::new(env2.clone(), NetworkId::UnitAlbatross).unwrap());

    for block in macro_blocks {
        assert_eq!(
            blockchain2.push_isolated_macro_block(block, &[]),
            Ok(PushResult::Extended)
        );
    }
}

// TODO Test transactions
