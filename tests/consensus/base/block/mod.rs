use core_rs::consensus::base::block::*;
use core_rs::consensus::base::primitive::hash::Blake2bHash;
use beserial::{Serialize, Deserialize};
use hex;

const MAINNET_GENESIS: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d9";

#[test]
fn it_can_deserialize_block_header() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS).unwrap();
    let bh: BlockHeader = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(bh.version, 1);
    assert_eq!(bh.prev_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(bh.interlink_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(bh.body_hash, Blake2bHash::from("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a"));
    assert_eq!(bh.accounts_hash, Blake2bHash::from("1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c"));
    // assert_eq!(bh.n_bits, 520159232); // TODO
    assert_eq!(bh.height, 1);
    assert_eq!(bh.timestamp, 1523727000);
    assert_eq!(bh.nonce, 137689);
}

#[test]
fn it_can_serialize_block_header() {
    let v: Vec<u8> = hex::decode(MAINNET_GENESIS).unwrap();
    let bh: BlockHeader = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(bh.serialized_size());
    let size = bh.serialize(&mut v2).unwrap();
    assert_eq!(size, bh.serialized_size());
    assert_eq!(hex::encode(v2), MAINNET_GENESIS);
}
