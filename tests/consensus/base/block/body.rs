use beserial::{Deserialize, Serialize};
use core_rs::consensus::base::block::*;
use core_rs::consensus::base::primitive::Address;
use core_rs::consensus::base::primitive::hash::{Blake2bHash, Hash};
use hex;

const MAINNET_GENESIS_BODY: &str = "0000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";

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
fn it_can_calculate_genesis_body_hash() {
    let body = BlockBody::deserialize_from_vec(&hex::decode(MAINNET_GENESIS_BODY).unwrap()).unwrap();
    assert_eq!(Hash::hash::<Blake2bHash>(&body).as_bytes(), &hex::decode("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a").unwrap()[..]);
}
