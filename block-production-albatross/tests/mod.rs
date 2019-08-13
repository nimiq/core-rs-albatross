use std::sync::Arc;

use beserial::Deserialize;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_block_albatross::{Block, MacroBlock, MacroExtrinsics, PbftProposal, PbftProofBuilder, PbftPrepareMessage, PbftCommitMessage, SignedPbftPrepareMessage, SignedPbftCommitMessage};
use nimiq_blockchain_albatross::blockchain::{Blockchain, PushResult};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_primitives::{networks::NetworkId};
use nimiq_primitives::policy;
use nimiq_blockchain_base::AbstractBlockchain;

const SECRET_KEY: &'static str = "49ea68eb6b8afdf4ca4d4c0a0b295c76ca85225293693bc30e755476492b707f";

#[test]
fn it_can_produce_empty_micro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::DevAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let keypair = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    let block = producer.next_micro_block(vec![], 1565713920000, 0, vec![0x41], None);
    assert_eq!(blockchain.push(Block::Micro(block)), Ok(PushResult::Extended));

    let block = producer.next_micro_block(vec![], 1565713922000, 0, vec![0x41], None);
    assert_eq!(blockchain.push(Block::Micro(block)), Ok(PushResult::Extended));
}

// Fill epoch with micro blocks
fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    let init_height = blockchain.head_height();
    let macro_block_number = policy::macro_block_of(init_height + 1);
    for _ in (init_height + 1)..macro_block_number {
        let last_micro_block = producer.next_micro_block(vec![], 1565713924000, 0, vec![0x42], None);
        assert_eq!(blockchain.push(Block::Micro(last_micro_block)), Ok(PushResult::Extended));
    }
    assert_eq!(blockchain.head_height(), macro_block_number - 1);
}

fn sign_macro_block(proposal: PbftProposal, extrinsics: MacroExtrinsics) -> MacroBlock {
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
        extrinsics: Some(extrinsics)
    }
}

#[test]
fn it_can_produce_macro_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::DevAlbatross).unwrap());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let keypair = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(Arc::clone(&blockchain), mempool, keypair);

    fill_micro_blocks(&producer, &blockchain);

    let proposal = producer.next_macro_block_proposal(1565713920000u64, 3u32, None);
    let extrinsics = producer.next_macro_extrinsics();

    let block = sign_macro_block(proposal, extrinsics);
    assert_eq!(blockchain.push(Block::Macro(block)), Ok(PushResult::Extended));
}

// TODO Test view changes
// TODO Test fork proofs
// TODO Test transactions
