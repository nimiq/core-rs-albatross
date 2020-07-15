use std::str::FromStr;

use beserial::{Deserialize, Serialize};
use fixed_unsigned::types::FixedUnsigned10;
use nimiq_block::{Block, Difficulty};
use nimiq_blockchain::{chain_info::ChainInfo, super_block_counts::SuperBlockCounts};
use nimiq_network_primitives::networks::NetworkInfo;
use nimiq_primitives::networks::NetworkId;

#[test]
fn it_is_correctly_initialized() {
    let genesis_block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let chain_info = ChainInfo::initial(genesis_block.clone());
    let mut super_block_counts = SuperBlockCounts::default();
    super_block_counts.add(0); // Depth for target is 0
    assert_eq!(chain_info.head, genesis_block);
    assert_eq!(chain_info.total_difficulty, Difficulty::from(1));
    assert_eq!(
        FixedUnsigned10::from(chain_info.total_work),
        FixedUnsigned10::from_str("1.8842573476").unwrap()
    );
    assert_eq!(chain_info.on_main_chain, true);
    assert_eq!(chain_info.main_chain_successor, None);
    assert_eq!(chain_info.super_block_counts, super_block_counts);
}

#[test]
fn it_can_be_serialized_and_deserialized() {
    let mut genesis_block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    genesis_block.body = None;
    let chain_info = ChainInfo::initial(genesis_block);

    let mut v: Vec<u8> = Vec::with_capacity(chain_info.serialized_size());
    chain_info.serialize(&mut v).unwrap();
    let chain_info2: ChainInfo = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(chain_info, chain_info2);
}

#[test]
fn serialize_strips_body() {
    let genesis_block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let mut chain_info = ChainInfo::initial(genesis_block.clone());

    let mut v: Vec<u8> = Vec::with_capacity(chain_info.serialized_size());
    chain_info.serialize(&mut v).unwrap();
    let chain_info2: ChainInfo = Deserialize::deserialize(&mut &v[..]).unwrap();

    chain_info.head.body = None;
    assert_eq!(chain_info, chain_info2);
}

#[test]
fn it_calculates_successor_correctly() {
    let genesis_block = NetworkInfo::from_network_id(NetworkId::Main)
        .genesis_block::<Block>()
        .clone();
    let chain_info = ChainInfo::initial(genesis_block.clone());
    let next_info = chain_info.next(genesis_block.clone());
    let mut super_block_counts = SuperBlockCounts::default();
    super_block_counts.add(0); // Depth for target is 0
    super_block_counts.add(0); // Two genesis blocks means two superblocks at depth 0
    assert_eq!(next_info.head, genesis_block);
    assert_eq!(next_info.total_difficulty, Difficulty::from(2));
    assert_eq!(
        FixedUnsigned10::from(next_info.total_work),
        FixedUnsigned10::from_str("3.7685146952").unwrap()
    );
    assert_eq!(next_info.on_main_chain, false);
    assert_eq!(next_info.main_chain_successor, None);
    assert_eq!(next_info.super_block_counts, super_block_counts);
}
