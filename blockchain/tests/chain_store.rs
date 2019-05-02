use nimiq_blockchain::{chain_info::ChainInfo, chain_store::ChainStore};
use nimiq_database::{volatile::VolatileEnvironment, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_primitives::networks::{get_network_info, NetworkId};
use nimiq_block::Difficulty;

#[test]
fn it_can_store_the_chain_head() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);
    assert!(store.get_head(None).is_none());

    let mut txn = WriteTransaction::new(&env);
    let head = Blake2bHash::from([1u8; Blake2bHash::SIZE]);
    store.set_head(&mut txn, &head);
    txn.commit();

    assert_eq!(store.get_head(None).unwrap(), head);
}

#[test]
fn it_can_store_chain_info_with_body() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);
    let genesis_block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let genesis_hash = genesis_block.header.hash();
    let chain_info = ChainInfo::initial(genesis_block);
    assert!(chain_info.head.body.is_some());

    let mut txn = WriteTransaction::new(&env);
    store.put_chain_info(&mut txn, &genesis_hash, &chain_info, true);
    txn.commit();

    let mut chain_info_no_body = chain_info.clone();
    chain_info_no_body.head.body = None;
    assert_eq!(store.get_chain_info(&genesis_hash, true, None).unwrap(), chain_info);
    assert_eq!(store.get_chain_info(&genesis_hash, false, None).unwrap(), chain_info_no_body);
    assert_eq!(store.get_block(&genesis_hash, true, None).unwrap(), chain_info.head);
    assert_eq!(store.get_block(&genesis_hash, false, None).unwrap(), chain_info_no_body.head);
}

#[test]
fn it_can_store_chain_info_without_body() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);
    let genesis_block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let genesis_hash = genesis_block.header.hash();
    let chain_info = ChainInfo::initial(genesis_block);

    let mut txn = WriteTransaction::new(&env);
    store.put_chain_info(&mut txn, &genesis_hash, &chain_info, false);
    txn.commit();

    let mut chain_info_no_body = chain_info.clone();
    chain_info_no_body.head.body = None;
    assert_eq!(store.get_chain_info(&genesis_hash, false, None).unwrap(), chain_info_no_body);
    assert_eq!(store.get_chain_info(&genesis_hash, true, None).unwrap(), chain_info_no_body);
    assert_eq!(store.get_block(&genesis_hash, false, None).unwrap(), chain_info_no_body.head);
    assert_eq!(store.get_block(&genesis_hash, true, None), None);
}

#[test]
fn it_can_retrieve_chain_info_by_height() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);

    let block1 = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let hash1 = block1.header.hash::<Blake2bHash>();
    let info1 = ChainInfo::initial(block1.clone());

    let mut block2 = block1.clone();
    block2.header.height = 5;
    let hash2 = block2.header.hash::<Blake2bHash>();
    let mut info2 = ChainInfo::initial(block2);
    info2.total_difficulty = Difficulty::from(4711);

    let mut block3_1 = block1.clone();
    block3_1.header.height = 28883;
    block3_1.header.interlink_hash = [2u8; Blake2bHash::SIZE].into();
    let hash3_1 = block3_1.header.hash::<Blake2bHash>();
    let mut info3_1 = ChainInfo::initial(block3_1);
    info3_1.on_main_chain = false;

    let mut block3_2 = block1.clone();
    block3_2.header.height = 28883;
    block3_2.header.interlink_hash = [5u8; Blake2bHash::SIZE].into();
    let hash3_2 = block3_2.header.hash::<Blake2bHash>();
    let mut info3_2 = ChainInfo::initial(block3_2);
    info3_2.main_chain_successor = Some(Blake2bHash::from([1u8; Blake2bHash::SIZE]));

    let mut txn = WriteTransaction::new(&env);
    store.put_chain_info(&mut txn, &hash1, &info1, true);
    store.put_chain_info(&mut txn, &hash2, &info2, false);
    store.put_chain_info(&mut txn, &hash3_1, &info3_1, true);
    store.put_chain_info(&mut txn, &hash3_2, &info3_2, true);
    txn.commit();

    let mut info2_no_body = info2.clone();
    info2_no_body.head.body = None;

    assert_eq!(store.get_chain_info_at(1, true, None).unwrap(), info1);
    assert_eq!(store.get_chain_info_at(5, false, None).unwrap(), info2_no_body);
    assert_eq!(store.get_chain_info_at(5, true, None).unwrap(), info2_no_body);
    assert_eq!(store.get_chain_info_at(28883, true, None).unwrap(), info3_2);

    let mut info3_2_no_body = info3_2.clone();
    info3_2_no_body.head.body = None;
    assert_eq!(store.get_chain_info_at(28883, false, None).unwrap(), info3_2_no_body);
}

#[test]
fn it_can_get_blocks_backward() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);

    let mut txn = WriteTransaction::new(&env);
    let mut block = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    store.put_chain_info(&mut txn, &block.header.hash::<Blake2bHash>(), &ChainInfo::initial(block.clone()), true);

    for _ in 0..20 {
        let mut b = block.clone();
        b.header.prev_hash = block.header.hash();
        b.header.height = block.header.height + 1;
        let hash = b.header.hash::<Blake2bHash>();
        store.put_chain_info(&mut txn, &hash, &ChainInfo::initial(b.clone()), true);
        block = b;
    }
    txn.commit();

    let mut blocks = store.get_blocks_backward(&block.header.prev_hash, 10, true, None);
    assert_eq!(blocks.len(), 10);
    assert_eq!(blocks[0].header.height, 19);
    assert_eq!(blocks[9].header.height, 10);
    assert!(blocks[0].body.is_some());
    assert!(blocks[9].body.is_some());

    blocks = store.get_blocks_backward(&block.header.prev_hash, 18, false, None);
    assert_eq!(blocks.len(), 18);
    assert_eq!(blocks[0].header.height, 19);
    assert_eq!(blocks[17].header.height, 2);
    assert!(blocks[0].body.is_none());
    assert!(blocks[17].body.is_none());

    blocks = store.get_blocks_backward(&block.header.prev_hash, 19, true, None);
    assert_eq!(blocks.len(), 19);
    assert_eq!(blocks[0].header.height, 19);
    assert_eq!(blocks[18].header.height, 1);
    assert!(blocks[0].body.is_some());
    assert!(blocks[18].body.is_some());

    blocks = store.get_blocks_backward(&block.header.prev_hash, 20, false, None);
    assert_eq!(blocks.len(), 19);
    assert_eq!(blocks[0].header.height, 19);
    assert_eq!(blocks[18].header.height, 1);
    assert!(blocks[0].body.is_none());
    assert!(blocks[18].body.is_none());

    blocks = store.get_blocks_backward(&block.header.hash::<Blake2bHash>(), 20, false, None);
    assert_eq!(blocks.len(), 20);
    assert_eq!(blocks[0].header.height, 20);
    assert_eq!(blocks[19].header.height, 1);

    blocks = store.get_blocks_backward(&block.header.hash::<Blake2bHash>(), 20, false, None);
    assert_eq!(blocks.len(), 20);
    assert_eq!(blocks[0].header.height, 20);
    assert_eq!(blocks[19].header.height, 1);

    blocks = store.get_blocks_backward(&blocks[18].header.hash::<Blake2bHash>(), 20, true, None);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].header.height, 1);
    assert!(blocks[0].body.is_some());

    blocks = store.get_blocks_backward(&blocks[0].header.hash::<Blake2bHash>(), 20, false, None);
    assert_eq!(blocks.len(), 0);
}

#[test]
fn it_can_get_blocks_forward() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);

    let mut txn = WriteTransaction::new(&env);
    let network_info = get_network_info(NetworkId::Main).unwrap();
    let mut block = network_info.genesis_block.clone();
    let mut chain_infos = vec![ChainInfo::initial(block.clone())];

    let mut hash;
    for _ in 0..20 {
        let mut b = block.clone();

        b.header.prev_hash = block.header.hash();
        b.header.height = block.header.height + 1;
        hash = b.header.hash::<Blake2bHash>();
        chain_infos.last_mut().unwrap().main_chain_successor = Some(hash);
        chain_infos.push(ChainInfo::initial(b.clone()));
        block = b;
    }

    for chain_info in chain_infos.iter() {
        let hash = chain_info.head.header.hash();
        store.put_chain_info(&mut txn, &hash, chain_info, true);
    }
    txn.commit();

    let second_block_hash = chain_infos.first().unwrap().main_chain_successor.as_ref().unwrap();

    let mut blocks = store.get_blocks_forward(second_block_hash, 10, true, None);
    assert_eq!(blocks.len(), 10);
    assert_eq!(blocks[0].header.height, 3);
    assert_eq!(blocks[9].header.height, 12);
    assert!(blocks[0].body.is_some());
    assert!(blocks[9].body.is_some());

    blocks = store.get_blocks_forward(second_block_hash, 18, false, None);
    assert_eq!(blocks.len(), 18);
    assert_eq!(blocks[0].header.height, 3);
    assert_eq!(blocks[17].header.height, 20);
    assert!(blocks[0].body.is_none());
    assert!(blocks[17].body.is_none());

    blocks = store.get_blocks_forward(second_block_hash, 19, true, None);
    assert_eq!(blocks.len(), 19);
    assert_eq!(blocks[0].header.height, 3);
    assert_eq!(blocks[18].header.height, 21);
    assert!(blocks[0].body.is_some());
    assert!(blocks[18].body.is_some());

    blocks = store.get_blocks_forward(second_block_hash, 20, false, None);
    assert_eq!(blocks.len(), 19);
    assert_eq!(blocks[0].header.height, 3);
    assert_eq!(blocks[18].header.height, 21);
    assert!(blocks[0].body.is_none());
    assert!(blocks[18].body.is_none());

    blocks = store.get_blocks_forward(&network_info.genesis_hash, 20, false, None);
    assert_eq!(blocks.len(), 20);
    assert_eq!(blocks[0].header.height, 2);
    assert_eq!(blocks[19].header.height, 21);

    blocks = store.get_blocks_forward(&network_info.genesis_hash, 20, false, None);
    assert_eq!(blocks.len(), 20);
    assert_eq!(blocks[0].header.height, 2);
    assert_eq!(blocks[19].header.height, 21);

    blocks = store.get_blocks_forward(&chain_infos[19].head.header.hash(), 20, true, None);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].header.height, 21);
    assert!(blocks[0].body.is_some());

    blocks = store.get_blocks_forward(&chain_infos[20].head.header.hash(), 20, false, None);
    assert_eq!(blocks.len(), 0);
}

#[test]
fn it_can_remove_chain_info() {
    let env = VolatileEnvironment::new(3).unwrap();
    let store = ChainStore::new(&env);

    let block1 = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let hash1 = block1.header.hash::<Blake2bHash>();
    let info1 = ChainInfo::initial(block1.clone());

    let mut block2_1 = block1.clone();
    block2_1.header.height = 28883;
    block2_1.header.interlink_hash = [2u8; Blake2bHash::SIZE].into();
    let hash2_1 = block2_1.header.hash::<Blake2bHash>();
    let mut info2_1 = ChainInfo::initial(block2_1);
    info2_1.on_main_chain = false;

    let mut block2_2 = block1.clone();
    block2_2.header.height = 28883;
    block2_2.header.interlink_hash = [5u8; Blake2bHash::SIZE].into();
    let hash2_2 = block2_2.header.hash::<Blake2bHash>();
    let mut info2_2 = ChainInfo::initial(block2_2);
    info2_2.main_chain_successor = Some(Blake2bHash::from([1u8; Blake2bHash::SIZE]));

    let mut txn = WriteTransaction::new(&env);
    store.put_chain_info(&mut txn, &hash1, &info1, true);
    store.put_chain_info(&mut txn, &hash2_1, &info2_1, true);
    store.put_chain_info(&mut txn, &hash2_2, &info2_2, true);
    txn.commit();

    assert_eq!(store.get_chain_info_at(1, true, None).unwrap(), info1);
    assert_eq!(store.get_chain_info(&hash1, true, None).unwrap(), info1);
    txn = WriteTransaction::new(&env);
    store.remove_chain_info(&mut txn, &hash1, 1);
    txn.commit();
    assert!(store.get_chain_info_at(1, true, None).is_none());
    assert!(store.get_chain_info(&hash1, true, None).is_none());

    assert_eq!(store.get_chain_info_at(28883, true, None).unwrap(), info2_2);
    assert_eq!(store.get_chain_info(&hash2_1, true, None).unwrap(), info2_1);
    assert_eq!(store.get_chain_info(&hash2_2, true, None).unwrap(), info2_2);
    txn = WriteTransaction::new(&env);
    store.remove_chain_info(&mut txn, &hash2_1, 28883);
    txn.commit();
    assert_eq!(store.get_chain_info_at(28883, true, None).unwrap(), info2_2);
    assert!(store.get_chain_info(&hash2_1, true, None).is_none());
    assert_eq!(store.get_chain_info(&hash2_2, true, None).unwrap(), info2_2);
}
