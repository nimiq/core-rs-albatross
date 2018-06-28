use beserial::{Deserialize, Serialize};
use core_rs::consensus::base::block::*;
use core_rs::consensus::base::primitive::Address;
use core_rs::consensus::base::primitive::hash::Blake2bHash;
use hex;

const MAINNET_GENESIS_HEADER: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d9";
const MAINNET_GENESIS_BODY: &str = "0000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";
const MAINNET_GENESIS_BLOCK: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d900010000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";
const BLOCK_108273: &str = "00012c2709b842ffd5d822fb881235a9981dcb4f031562437bbff4641d09e5bfcd11b14f16b209ac32a3d909adcc759b2c53f8f9f478df7dcb74101c57448af2993f8026c8d5f600afa7c0cea5d2163acbd97ba96de9d1cc82007840c06451b6894b913cfeb96831cda39e7a49b5c6af7fb3a5c0b86fd544c78e653e275b7e180f791c20ce8c0001a6f15b3549ec9728d73011c85c00e422eb357417aff6c0b474e682f93b655359494cfebb2cd6b8fa2e9fb3f222268edaecae80ab0404f03984e31f19d696a225f11abbd99bf04f0ee95266dfe32cc81df55a834afd5e2b030cecb9098dc11afcda1d193a11932295ee7643947dd4bf5605eac12bd307ccc00ac55d591d5aeaf62d6b4bf7861f55791525a41d86b970bfa2bf633a1c681a75fb898331fbedea826aee38adcc63b178a504c1f4c7cf3ddb1d7e71258d31ef411f905d249d44b524494360a95cf4705ec7aa52f62e8d8430ab5bfaf2433ce9af81ca4810c98b54c6b2ecdf2c28f043143a18c74ff445805c8084bc95ffa368380978c9a44945a7ffeb053f687f58e5c77d2dc08d46fb7bce4aa80ca24f04870b31bb5f4a4e21d143e21d33e7bcecfa232db3ccf691175d5485f031c039a43f641839186a4716866e655d89c85af79754ad8786c08df001432715a84417723b0889b22b6917de391ab2fa480d736b79706f6f6c2d7368312d3200020042b32159040ebe741316b18107564b28783e89582edc2173917b38ab39d904ee47d18f75e7bc7036e0bb496252acd74f0bf8c6450000000000325aa000000000000000000001a6ef2a1ff786d2f42ad64bd97f139224a5a51f0260c4accce427e14cecb89c87711bbd6a7e7c26fde70eac3c555d8c705b977ce303a13b940a13a3a7470bc2f30d550000fe9bb3282823f79cb03b3b8de3f5fb6e2c66f1959f598442caaf0a680998bccbc1fb2d53a6d7e0011e85c7f81f5b7a76088c154a00000000000cc79000000000000000000001a6ef2aa7eb223b1495cce947e8c6e5113d71c5bf78cb2d6d039db1d374d0fa5954073e8f003b60dc5791a0e20f7ad9ab9df68ffe825d6f8f01ee2d381eee15f5728b0b0000";

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

#[test]
fn it_can_deserialize_block_108273() {
    let v: Vec<u8> = hex::decode(BLOCK_108273).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(block.header.version, 1);
    assert_eq!(block.header.prev_hash, Blake2bHash::from("2c2709b842ffd5d822fb881235a9981dcb4f031562437bbff4641d09e5bfcd11"));
    assert_eq!(block.header.interlink_hash, Blake2bHash::from("b14f16b209ac32a3d909adcc759b2c53f8f9f478df7dcb74101c57448af2993f"));
    assert_eq!(block.header.body_hash, Blake2bHash::from("8026c8d5f600afa7c0cea5d2163acbd97ba96de9d1cc82007840c06451b6894b"));
    assert_eq!(block.header.accounts_hash, Blake2bHash::from("913cfeb96831cda39e7a49b5c6af7fb3a5c0b86fd544c78e653e275b7e180f79"));
    assert_eq!(block.header.n_bits, 471912076.into());
    assert_eq!(block.header.height, 108273);
    assert_eq!(block.header.timestamp, 1530218988);
    assert_eq!(block.header.nonce, 2536036144);
    assert_eq!(block.interlink.len(), 17);
    if let Option::Some(ref body) = block.body {
        assert_eq!(body.miner, Address::from(&hex::decode("432715a84417723b0889b22b6917de391ab2fa48").unwrap()[..]));
        assert_eq!(body.extra_data, hex::decode("736b79706f6f6c2d7368312d32").unwrap());
        assert_eq!(body.transactions.len(), 2);
        assert_eq!(body.pruned_accounts.len(), 0);
        assert_eq!(body.transactions[0].sender, Address::from(&hex::decode("8a3eed6de76d21fe6f5ef1a1a8f0f2c2c070c4f3").unwrap()[..]));
        assert_eq!(body.transactions[0].recipient, Address::from(&hex::decode("47d18f75e7bc7036e0bb496252acd74f0bf8c645").unwrap()[..]));
        assert_eq!(body.transactions[0].value, 3300000);
        assert_eq!(body.transactions[0].fee, 0);
        assert_eq!(body.transactions[0].validity_start_height, 108271);
        assert_eq!(body.transactions[1].sender, Address::from(&hex::decode("2dcf5d9271c2e80680c17d6d66b8a3f0f03f734a").unwrap()[..]));
        assert_eq!(body.transactions[1].recipient, Address::from(&hex::decode("c1fb2d53a6d7e0011e85c7f81f5b7a76088c154a").unwrap()[..]));
        assert_eq!(body.transactions[1].value, 837520);
        assert_eq!(body.transactions[1].fee, 0);
        assert_eq!(body.transactions[1].validity_start_height, 108271);
    } else {
        panic!("Body should be present");
    }
}

#[test]
fn it_can_serialize_block_108273() {
    let v: Vec<u8> = hex::decode(BLOCK_108273).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(block.serialized_size());
    let size = block.serialize(&mut v2).unwrap();
    assert_eq!(size, block.serialized_size());
    assert_eq!(hex::encode(v2), BLOCK_108273);
}
