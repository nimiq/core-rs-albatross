use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, MacroBlock, MacroExtrinsics, PbftCommitMessage, PbftPrepareMessage, PbftProofBuilder,
    PbftProposal, SignedPbftCommitMessage, SignedPbftPrepareMessage,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::blockchain::{Blockchain, PushResult};
use nimiq_blockchain_base::AbstractBlockchain;
use nimiq_blockchain_base::Direction;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::policy;

use nimiq_block_production_albatross::test_utils::*;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &'static str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

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
