use beserial::Deserialize;
use parking_lot::RwLock;
use std::sync::Arc;
use nimiq::consensus::base::block::Block;
use nimiq::consensus::base::blockchain::{Blockchain, PushResult};
use nimiq::consensus::base::mempool::Mempool;
use nimiq::consensus::networks::NetworkId;
use nimiq::network::NetworkTime;
use nimiq::utils::db::volatile::VolatileEnvironment;

const BLOCK_2: &str = "0001264aaf8a4f9828a76c550635da078eb466306a189fcc03710bee9f649c869d120492e3986e75ac0d1466b5d6a7694c86839767a30980f8ba0d8c6e48631bc9cdd8a3eb957567d76963ad10d11e65453f763928fb9619e5f396a0906e946cce3ca7fcbb5fb2e35055de071e868381ba426a8d79d97cb48dab8345baeb9a9abb091f010000000000025ad23a98000046fe0180010000000000000000000000000000000000000000184d696e65642077697468206c6f766520627920526963687900000000";
const BLOCK_3: &str = "0001bab534467866d83060b1af0b3493dd0f97d7071b16e1562cf4b18bdf73e71ccb4aa1fea2b8cdf2a63411776c6391a7659aef4dd25317a615499c7b461e9a0405385dbed68e76f74317cc6f4cd40db832eb71b8338fad024ddbb88f9abc79f199dd6a3500aeb5479eb460afeab3363783e243a6e551536c3c01c8fca21d7afbbb1f00fddd000000035ad23a980000968102c0010000000000000000000000000000000000000000184d696e65642077697468206c6f76652062792054616d6d6f00000000";
const BLOCK_4: &str = "0001622b0536bbe764a5723f17cde03d2fa2b67a3f42f7cab082c72222eb1e48db7a607f7686d7636b500cfa620567ede30a15a12f69e22d35dd004bbdbfcaefc12520428a900c8dfb339b99aebb1d14cc4d5cebedf562aa1806f272deecbf3c5263b62534d1cda41d1a7bf70a6850c6c82936adb9b2ef66b7421ca3c55664c1417f1f00fbb7000000045ad23a9800022dc60280bab534467866d83060b1af0b3493dd0f97d7071b16e1562cf4b18bdf73e71ccb0100000000000000000000000000000000000000001b4d696e65642077697468206c6f7665206279204372697374696e6100000000";
const BLOCK_5: &str = "000184d5a44ba5ae9961837e7fb19c176a19f77b2e0655873149017351e17b622cef4aa1fea2b8cdf2a63411776c6391a7659aef4dd25317a615499c7b461e9a0405b32082f43aae5c61bf1171e85650b550bcc2b8d020365619ecaeb924c4562770cbadc05e0c4117bf975bc3d7e55d2f3a13efe1a9baf17c0b2c3c42faee9414b31f00f98c000000055ad23a9800013f5602c0010000000000000000000000000000000000000000174d696e65642077697468206c6f7665206279204174756100000000";

#[test]
fn it_can_extend_the_main_chain() {
    crate::setup();

    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(RwLock::new(NetworkTime {})), NetworkId::Main));
    let mempool = Mempool::new(blockchain.clone());

    let mut block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    let mut status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_3).unwrap()).unwrap();
    status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_4).unwrap()).unwrap();
    status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_5).unwrap()).unwrap();
    status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);
}
