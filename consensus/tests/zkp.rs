use std::sync::Arc;

use futures::StreamExt;
use nimiq_block::Block;
use nimiq_genesis::NetworkInfo;
use nimiq_primitives::policy;
use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_consensus::zkp::zkp_component::{ZKPComponent, ZKPComponentEvent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_nano_primitives::MacroBlock as ZKPMacroBlock;
use nimiq_nano_zkp::NanoZKP;
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ))
}

#[test(tokio::test)]
#[ignore]
async fn can_produce_first_zkp_proof() {
    NanoZKP::setup().unwrap();
    let blockchain = blockchain();

    let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
    let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();
    let mut zkp_prover = ZKPComponent::new(Arc::clone(&blockchain), genesis_block.clone());

    assert_eq!(blockchain.read().block_number(), 0);
    let latest_pks: Vec<_> = genesis_block
        .get_validators()
        .unwrap()
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect();

    let genesis_block = ZKPMacroBlock::try_from(&genesis_block).unwrap();

    // Produce the 1st election block after genesis
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &blockchain, policy::BATCHES_PER_EPOCH as usize);

    // Waits for the proof generation and verifies the proof
    if let Some(ZKPComponentEvent::ElectionZKProof(block_number, hash, Ok(proof), pks)) =
        zkp_prover.next().await
    {
        let result = NanoZKP::verify(
            genesis_block.block_number,
            genesis_block.header_hash,
            latest_pks,
            block_number,
            hash.into(),
            pks,
            proof,
        );

        match result {
            Ok(valid) => assert!(valid, "ZK Proof for election block is invalid"),
            Err(e) => panic!("Error generating the zk proof {}", e),
        }
    }
}
