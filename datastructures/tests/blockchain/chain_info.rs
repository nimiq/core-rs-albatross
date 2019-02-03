use beserial::{Deserialize, Serialize};
use datastructures::blockchain::ChainInfo;
use network_primitives::networks::get_network_info;
use primitives::block::Difficulty;
use primitives::networks::NetworkId;

#[test]
fn it_is_correctly_initialized() {
    let genesis_block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let chain_info = ChainInfo::initial(genesis_block.clone());
    assert_eq!(chain_info.head, genesis_block);
    assert_eq!(chain_info.total_difficulty, genesis_block.header.n_bits.into());
    assert_eq!(chain_info.on_main_chain, true);
    assert_eq!(chain_info.main_chain_successor, None);
}

#[test]
fn it_can_be_serialized_and_deserialized() {
    let mut genesis_block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    genesis_block.body = None;
    let chain_info = ChainInfo::initial(genesis_block);

    let mut v: Vec<u8> = Vec::with_capacity(chain_info.serialized_size());
    chain_info.serialize(&mut v).unwrap();
    let chain_info2: ChainInfo = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(chain_info, chain_info2);
}

#[test]
fn serialize_strips_body() {
    let genesis_block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let mut chain_info = ChainInfo::initial(genesis_block.clone());

    let mut v: Vec<u8> = Vec::with_capacity(chain_info.serialized_size());
    chain_info.serialize(&mut v).unwrap();
    let chain_info2: ChainInfo = Deserialize::deserialize(&mut &v[..]).unwrap();

    chain_info.head.body = None;
    assert_eq!(chain_info, chain_info2);
}

#[test]
fn it_calculates_successor_correctly() {
    let genesis_block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let chain_info = ChainInfo::initial(genesis_block.clone());
    let next_info = chain_info.next(genesis_block.clone());
    assert_eq!(next_info.head, genesis_block);
    assert_eq!(next_info.total_difficulty, Difficulty::from(2));
    assert_eq!(next_info.on_main_chain, false);
    assert_eq!(next_info.main_chain_successor, None);
}
