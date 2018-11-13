use beserial::{Deserialize, Serialize};
use nimiq::consensus::base::block::*;
use nimiq::consensus::base::primitive::hash::{Argon2dHash, Blake2bHash, Hash};
use hex;

const MAINNET_GENESIS_HEADER: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d9";

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
fn it_can_calculate_genesis_header_hash() {
    let header = BlockHeader::deserialize_from_vec(&hex::decode(MAINNET_GENESIS_HEADER).unwrap()).unwrap();
    assert_eq!(Hash::hash::<Blake2bHash>(&header).as_bytes(), &hex::decode("264AAF8A4F9828A76C550635DA078EB466306A189FCC03710BEE9F649C869D12").unwrap()[..]);
}

#[test]
fn it_can_calculate_genesis_header_pow() {
    let header = BlockHeader::deserialize_from_vec(&hex::decode(MAINNET_GENESIS_HEADER).unwrap()).unwrap();
    assert_eq!(Hash::hash::<Argon2dHash>(&header).as_bytes(), &hex::decode("000087dccfcb8625a84c821887c5c515f92f5078501bf49369f8993657cdc034").unwrap()[..]);
}

#[test]
fn it_can_verify_proof_of_work() {
    let header = BlockHeader::deserialize_from_vec(&hex::decode(MAINNET_GENESIS_HEADER).unwrap()).unwrap();
    assert!(header.verify_proof_of_work());
}

#[test]
fn it_correctly_identifies_immediate_successors() {
    let header1 = BlockHeader::deserialize_from_vec(&hex::decode(MAINNET_GENESIS_HEADER).unwrap()).unwrap();
    let mut header2 = BlockHeader {
        version: 1,
        prev_hash: header1.hash(),
        interlink_hash: Blake2bHash::from([0u8; Blake2bHash::SIZE]),
        body_hash: Blake2bHash::from([0u8; Blake2bHash::SIZE]),
        accounts_hash: Blake2bHash::from([0u8; Blake2bHash::SIZE]),
        n_bits: 0x1f010000.into(),
        height: 2,
        timestamp: header1.timestamp + 1,
        nonce: 0
    };
    assert!(header2.is_immediate_successor_of(&header1));

    // header2.height == header1.height + 1
    header2.height = 3;
    assert!(!header2.is_immediate_successor_of(&header1));
    header2.height = 1;
    assert!(!header2.is_immediate_successor_of(&header1));
    header2.height = 2;

    // header2.timestamp >= header1.timestamp
    header2.timestamp = header1.timestamp;
    assert!(header2.is_immediate_successor_of(&header1));
    header2.timestamp = header1.timestamp - 1;
    assert!(!header2.is_immediate_successor_of(&header1));
    header2.timestamp = header1.timestamp + 1;

    // header2.prev_hash == header1.hash()
    header2.prev_hash = Blake2bHash::from([1u8; Blake2bHash::SIZE]);
    assert!(!header2.is_immediate_successor_of(&header1));

    header2.prev_hash = header1.hash();
    assert!(header2.is_immediate_successor_of(&header1));
}
