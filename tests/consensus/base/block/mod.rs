use core_rs::consensus::base::block::*;
use core_rs::consensus::base::primitive::Address;
use core_rs::consensus::base::primitive::hash::Blake2bHash;
use beserial::{Serialize, Deserialize};
use hex;

const MAINNET_GENESIS_HEADER: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d9";
const MAINNET_GENESIS_BODY: &str = "0000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";
const MAINNET_GENESIS_BLOCK: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d900010000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";

#[test]
fn it_can_deserialize_genesis_header() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS_HEADER).unwrap();
    let bh: BlockHeader = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(bh.version, 1);
    assert_eq!(bh.prev_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(bh.interlink_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(bh.body_hash, Blake2bHash::from("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a"));
    assert_eq!(bh.accounts_hash, Blake2bHash::from("1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c"));
    assert_eq!(bh.n_bits, 520159232.into());
    assert_eq!(bh.height, 1);
    assert_eq!(bh.timestamp, 1523727000);
    assert_eq!(bh.nonce, 137689);
}

#[test]
fn it_can_serialize_genesis_header() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS_HEADER).unwrap();
    let bh: BlockHeader = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(bh.serialized_size());
    let size = bh.serialize(&mut v2).unwrap();
    assert_eq!(size, bh.serialized_size());
    assert_eq!(hex::encode(v2), MAINNET_GENESIS_HEADER);
}

#[test]
fn it_can_deserialize_genesis_body() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(body.miner, Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]));
    assert_eq!(body.extra_data, "love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".as_bytes().to_vec());
    assert_eq!(body.transactions.len(), 0);
    assert_eq!(body.pruned_accounts.len(), 0);
}

#[test]
fn it_can_serialize_genesis_body() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(body.serialized_size());
    let size = body.serialize(&mut v2).unwrap();
    assert_eq!(size, body.serialized_size());
    assert_eq!(hex::encode(v2), MAINNET_GENESIS_BODY);
}

#[test]
fn it_can_deserialzie_genesis_block() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS_BLOCK).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(block.header.version, 1);
    assert_eq!(block.header.prev_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(block.header.interlink_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(block.header.body_hash, Blake2bHash::from("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a"));
    assert_eq!(block.header.accounts_hash, Blake2bHash::from("1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c"));
    assert_eq!(block.header.n_bits, 520159232.into());
    assert_eq!(block.header.height, 1);
    assert_eq!(block.header.timestamp, 1523727000);
    assert_eq!(block.header.nonce, 137689);
    assert_eq!(block.interlink.len(), 0);
    if let Option::Some(ref body) = block.body {
        assert_eq!(body.miner, Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]));
        assert_eq!(body.extra_data, "love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".as_bytes().to_vec());
        assert_eq!(body.transactions.len(), 0);
        assert_eq!(body.pruned_accounts.len(), 0);
    } else {
        panic!("Body should be present");
    }
}

#[test]
fn it_can_serialize_genesis_block() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS_BLOCK).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(block.serialized_size());
    let size = block.serialize(&mut v2).unwrap();
    assert_eq!(size, block.serialized_size());
    assert_eq!(hex::encode(v2), MAINNET_GENESIS_BLOCK);
}
