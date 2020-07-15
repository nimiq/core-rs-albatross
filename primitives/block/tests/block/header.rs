use hex;

use beserial::{Deserialize, Serialize};
use hash::{Argon2dHash, Blake2bHash, Hash};
use nimiq_block::*;

const GENESIS_HEADER: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d9";
const B108273_HEADER: &str = "00012c2709b842ffd5d822fb881235a9981dcb4f031562437bbff4641d09e5bfcd11b14f16b209ac32a3d909adcc759b2c53f8f9f478df7dcb74101c57448af2993f8026c8d5f600afa7c0cea5d2163acbd97ba96de9d1cc82007840c06451b6894b913cfeb96831cda39e7a49b5c6af7fb3a5c0b86fd544c78e653e275b7e180f791c20ce8c0001a6f15b3549ec9728d730";
const B169500_HEADER: &str = "0001e5ed6f0aad730ddaa2f17445ea3d93e05ca6484d672217e7bae5d6d9c46c71429647e98d1884240082fe16fb5d89c55dcb25f12895753884fab3922ff5bc945fa65795b408a23693203cb7bbae3206fafcf0f49a8e2832b51df27e904d1f397d3c02ddd12171844cabd7ed21eab37a66151a1393c7b4a4cc7f9b7a5b79de7ffe1c2a9f9e0002961c5b6d93a400023097";

#[test]
fn it_can_deserialize_genesis_header() {
    let v: Vec<u8> = hex::decode(GENESIS_HEADER).unwrap();
    let bh: BlockHeader = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(bh.version, 1);
    assert_eq!(
        bh.prev_hash,
        Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000")
    );
    assert_eq!(
        bh.interlink_hash,
        Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000")
    );
    assert_eq!(
        bh.body_hash,
        Blake2bHash::from("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a")
    );
    assert_eq!(
        bh.accounts_hash,
        Blake2bHash::from("1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c")
    );
    assert_eq!(bh.n_bits, 520159232.into());
    assert_eq!(bh.height, 1);
    assert_eq!(bh.timestamp, 1523727000);
    assert_eq!(bh.nonce, 137689);
}

#[test]
fn it_can_serialize_genesis_header() {
    let v: Vec<u8> = hex::decode(GENESIS_HEADER).unwrap();
    let bh: BlockHeader = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(bh.serialized_size());
    let size = bh.serialize(&mut v2).unwrap();
    assert_eq!(size, bh.serialized_size());
    assert_eq!(hex::encode(v2), GENESIS_HEADER);
}

#[test]
fn it_can_calculate_genesis_block_hashes() {
    let header = BlockHeader::deserialize_from_vec(&hex::decode(GENESIS_HEADER).unwrap()).unwrap();
    assert_eq!(
        header.hash::<Blake2bHash>(),
        Blake2bHash::from("264aaf8a4f9828a76c550635da078eb466306a189fcc03710bee9f649c869d12")
    );
    assert_eq!(
        header.hash::<Argon2dHash>(),
        Argon2dHash::from("000087dccfcb8625a84c821887c5c515f92f5078501bf49369f8993657cdc034")
    );
}

#[test]
fn it_can_calculate_block_108273_hash() {
    let header = BlockHeader::deserialize_from_vec(&hex::decode(B108273_HEADER).unwrap()).unwrap();
    assert_eq!(
        header.hash::<Blake2bHash>(),
        Blake2bHash::from("65631e10f76ac8e95ec0766e84ec2be46818e2351b0174220aab7fc7243fca17")
    );
    assert_eq!(
        header.hash::<Argon2dHash>(),
        Argon2dHash::from("00000000082480b610551e47995125087532c3ea3865d2846e53cf32184f01e4")
    );
}

#[test]
fn it_can_calculate_block_169500_hash() {
    let header = BlockHeader::deserialize_from_vec(&hex::decode(B169500_HEADER).unwrap()).unwrap();
    assert_eq!(
        header.hash::<Blake2bHash>(),
        Blake2bHash::from("3c084e90460d0313e87d9dfe3d4bafd74cab083d426529cc17e94df3be548f59")
    );
    assert_eq!(
        header.hash::<Argon2dHash>(),
        Argon2dHash::from("000000000c0d13dca4b49b3ecfb9a734fad2fc9ffea7d49ee8a6832fc4c6200e")
    );
}

#[test]
fn verify_accepts_valid_proof_of_work() {
    let header1 = BlockHeader::deserialize_from_vec(&hex::decode(GENESIS_HEADER).unwrap()).unwrap();
    assert!(header1.verify_proof_of_work());
    let header2 = BlockHeader::deserialize_from_vec(&hex::decode(B108273_HEADER).unwrap()).unwrap();
    assert!(header2.verify_proof_of_work());
    let header3 = BlockHeader::deserialize_from_vec(&hex::decode(B169500_HEADER).unwrap()).unwrap();
    assert!(header3.verify_proof_of_work());
}

#[test]
fn verify_rejects_invalid_proof_of_work() {
    let mut header1 =
        BlockHeader::deserialize_from_vec(&hex::decode(GENESIS_HEADER).unwrap()).unwrap();
    header1.nonce = 1;
    assert!(!header1.verify_proof_of_work());

    let mut header2: BlockHeader =
        BlockHeader::deserialize_from_vec(&hex::decode(B108273_HEADER).unwrap()).unwrap();
    header2.n_bits = 0x05010000u32.into();
    assert!(!header2.verify_proof_of_work());
}

#[test]
fn it_correctly_identifies_immediate_successors() {
    let header1 = BlockHeader::deserialize_from_vec(&hex::decode(GENESIS_HEADER).unwrap()).unwrap();
    let mut header2 = BlockHeader {
        version: 1,
        prev_hash: header1.hash(),
        interlink_hash: Blake2bHash::from([0u8; Blake2bHash::SIZE]),
        body_hash: Blake2bHash::from([0u8; Blake2bHash::SIZE]),
        accounts_hash: Blake2bHash::from([0u8; Blake2bHash::SIZE]),
        n_bits: 0x1f010000.into(),
        height: 2,
        timestamp: header1.timestamp + 1,
        nonce: 0,
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
