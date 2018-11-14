use beserial::{Deserialize, Serialize};
use nimiq::consensus::base::block::*;
use nimiq::consensus::base::primitive::{Address, Coin};
use nimiq::consensus::base::primitive::hash::{Hash, Blake2bHash, Argon2dHash};
use nimiq::consensus::networks::NetworkId;
use hex;

const GENESIS_BLOCK: &str = "0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c1f010000000000015ad23a98000219d900010000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";
const BLOCK_108273: &str = "00012c2709b842ffd5d822fb881235a9981dcb4f031562437bbff4641d09e5bfcd11b14f16b209ac32a3d909adcc759b2c53f8f9f478df7dcb74101c57448af2993f8026c8d5f600afa7c0cea5d2163acbd97ba96de9d1cc82007840c06451b6894b913cfeb96831cda39e7a49b5c6af7fb3a5c0b86fd544c78e653e275b7e180f791c20ce8c0001a6f15b3549ec9728d73011c85c00e422eb357417aff6c0b474e682f93b655359494cfebb2cd6b8fa2e9fb3f222268edaecae80ab0404f03984e31f19d696a225f11abbd99bf04f0ee95266dfe32cc81df55a834afd5e2b030cecb9098dc11afcda1d193a11932295ee7643947dd4bf5605eac12bd307ccc00ac55d591d5aeaf62d6b4bf7861f55791525a41d86b970bfa2bf633a1c681a75fb898331fbedea826aee38adcc63b178a504c1f4c7cf3ddb1d7e71258d31ef411f905d249d44b524494360a95cf4705ec7aa52f62e8d8430ab5bfaf2433ce9af81ca4810c98b54c6b2ecdf2c28f043143a18c74ff445805c8084bc95ffa368380978c9a44945a7ffeb053f687f58e5c77d2dc08d46fb7bce4aa80ca24f04870b31bb5f4a4e21d143e21d33e7bcecfa232db3ccf691175d5485f031c039a43f641839186a4716866e655d89c85af79754ad8786c08df001432715a84417723b0889b22b6917de391ab2fa480d736b79706f6f6c2d7368312d3200020042b32159040ebe741316b18107564b28783e89582edc2173917b38ab39d904ee47d18f75e7bc7036e0bb496252acd74f0bf8c6450000000000325aa000000000000000000001a6ef2a1ff786d2f42ad64bd97f139224a5a51f0260c4accce427e14cecb89c87711bbd6a7e7c26fde70eac3c555d8c705b977ce303a13b940a13a3a7470bc2f30d550000fe9bb3282823f79cb03b3b8de3f5fb6e2c66f1959f598442caaf0a680998bccbc1fb2d53a6d7e0011e85c7f81f5b7a76088c154a00000000000cc79000000000000000000001a6ef2aa7eb223b1495cce947e8c6e5113d71c5bf78cb2d6d039db1d374d0fa5954073e8f003b60dc5791a0e20f7ad9ab9df68ffe825d6f8f01ee2d381eee15f5728b0b0000";
const BLOCK_169500: &str = "0001e5ed6f0aad730ddaa2f17445ea3d93e05ca6484d672217e7bae5d6d9c46c71429647e98d1884240082fe16fb5d89c55dcb25f12895753884fab3922ff5bc945fa65795b408a23693203cb7bbae3206fafcf0f49a8e2832b51df27e904d1f397d3c02ddd12171844cabd7ed21eab37a66151a1393c7b4a4cc7f9b7a5b79de7ffe1c2a9f9e0002961c5b6d93a40002309711d7158085c2b5c21a61184baa85b45dce5f01efb1a7ea406d5a03321e88d5591203facc187ed1f71ae10f387fa470a03b35aa8b5cec149b30677bb1d7f31ee2804712798ebb63ebe968352b3808f08ab03074a2176b13219b6358ed5b25030de1bfe4b9226e3e7c6ae737ee5be7f63c8c6db08fac57068cd69cb5bbca6c9f12fbecf9cefd0b89122748de57ea59b282e10d64b2243ea57b6ff0d8178eeb2bb256c687f889cab12264f6a5320fc74ec9b18275d3db6da35a7a4dcd7c6b0208f49511f84f97475b01e9c763c07fad35245ca795dff12cd162b0b4a40aa108b831e36ba0a201b403040ff02b4a9a4d10f8a4beff71750cd3aa7f324e696d6275732d393630656364346238313936303030303030323266386531313034663066663830323530373939373632320003010000ad8e224835e6cc0cadbcf500a49dae46f67697040224786862babbdb05e7c4430612135eb2a836812300000000000000000100000000000000000002961b2a0000a4010301daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f94dc65fbd91fc9f5ed1d92dba86d2e59607774b13e27a556d92a56ade9808d32cebdb4e704995dd63f97429a827541d3c6253bc0c2f472c3341b538e2f4bc60800e930b6408b84cb3a7985c54e742c4a1af3110b865386144a0e1badd0bceefc162cecfda0761ce11e06a076c2882b2e21309602973a4443ba4d672b665d4f3f0100bf703dd4eb71e245cced26f014f40dc29f0f71e6bffce722a7ceecba1b71cb8d44d5c0727a1a08efd376cd005293e33236232fad0000000000004e2000000000000001180002961a2a8f6693b1eb1537aa8eec3f99898d91474e5cfa1de1c81a1ee9e0c04ba0205e310d65708ea6e5ed9e7953f4dce64e73392333a2b83d1b30d0df9eb74bcc68980900725323e8f226e7b9dac90331505844f83e45f28cc6f0534039f55c725536d1ec6573413bec835fa4fb54f4dfc3b273a9e8ecc95b000000000011c7020000000000000000000296192a53ddbfd2dd72a78a8b62699e0012585f4768f07e397ae3bb23d0cd379c0c12c9dbe0aca1ae8f005330bbb33c80ee1449229ad6d0e482a1fc8124233dd953600e0001ad8e224835e6cc0cadbcf500a49dae46f67697040200000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f01000296350000000000000001";

#[test]
fn it_can_deserialize_genesis_block() {
    let v: Vec<u8> = hex::decode(GENESIS_BLOCK).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(block.header.version, 1);
    assert_eq!(block.header.prev_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(block.header.interlink_hash, Blake2bHash::from("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(block.header.body_hash, Blake2bHash::from("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a"));
    assert_eq!(block.header.accounts_hash, Blake2bHash::from("1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c"));
    assert_eq!(block.header.n_bits, 0x1f010000.into());
    assert_eq!(block.header.height, 1);
    assert_eq!(block.header.timestamp, 1523727000);
    assert_eq!(block.header.nonce, 137689);
    assert_eq!(block.interlink.len(), 0);
    if let Option::Some(ref body) = block.body {
        assert_eq!(body.miner, Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]));
        assert_eq!(
            body.extra_data,
            "love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".as_bytes().to_vec()
        );
        assert_eq!(body.transactions.len(), 0);
        assert_eq!(body.pruned_accounts.len(), 0);
    } else {
        panic!("Body should be present");
    }
}

#[test]
fn it_can_serialize_genesis_block() {
    let v: Vec<u8> = hex::decode(GENESIS_BLOCK).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(block.serialized_size());
    let size = block.serialize(&mut v2).unwrap();
    assert_eq!(size, block.serialized_size());
    assert_eq!(hex::encode(v2), GENESIS_BLOCK);
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
        assert_eq!(body.transactions[0].value, Coin::from(3300000));
        assert_eq!(body.transactions[0].fee, Coin::ZERO);
        assert_eq!(body.transactions[0].validity_start_height, 108271);
        assert_eq!(body.transactions[1].sender, Address::from(&hex::decode("2dcf5d9271c2e80680c17d6d66b8a3f0f03f734a").unwrap()[..]));
        assert_eq!(body.transactions[1].recipient, Address::from(&hex::decode("c1fb2d53a6d7e0011e85c7f81f5b7a76088c154a").unwrap()[..]));
        assert_eq!(body.transactions[1].value, Coin::from(837520));
        assert_eq!(body.transactions[1].fee, Coin::ZERO);
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

#[test]
fn it_can_serialize_block_169500() {
    let v: Vec<u8> = hex::decode(BLOCK_169500).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(block.serialized_size());
    let size = block.serialize(&mut v2).unwrap();
    assert_eq!(size, block.serialized_size());
    assert_eq!(hex::encode(v2), BLOCK_169500);
}


#[test]
fn verify_accepts_genesis_block() {
    let v: Vec<u8> = hex::decode(GENESIS_BLOCK).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert!(block.verify(block.header.timestamp, NetworkId::Main).is_ok());
}

#[test]
fn verify_accepts_block_108273() {
    let v: Vec<u8> = hex::decode(BLOCK_108273).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert!(block.verify(block.header.timestamp, NetworkId::Main).is_ok());
}

#[test]
fn verify_accepts_block_169500() {
    let v: Vec<u8> = hex::decode(BLOCK_169500).unwrap();
    let block: Block = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert!(block.verify(block.header.timestamp, NetworkId::Main).is_ok());
}

#[test]
fn verify_rejects_unsupported_block_versions() {
    let mut block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    block.header.version = 69;
    assert_eq!(block.verify(block.header.timestamp, NetworkId::Main), Err(BlockError::UnsupportedVersion));
}

#[test]
fn verify_rejects_blocks_from_the_future() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    assert_eq!(block.verify(block.header.timestamp - 2000, NetworkId::Main), Err(BlockError::FromTheFuture));
}

#[test]
fn verify_rejects_invalid_pow() {
    let mut block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    block.header.nonce = 1;
    assert_eq!(block.verify(block.header.timestamp, NetworkId::Main), Err(BlockError::InvalidPoW));
}

#[test]
fn verify_rejects_excessive_size() {
    let mut block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    {
        let body = block.body.as_mut().unwrap();
        let tx = body.transactions[0].clone();
        for _ in 0..800 {
            body.transactions.push(tx.clone());
        }
    }
    assert_eq!(block.verify(block.header.timestamp, NetworkId::Main), Err(BlockError::SizeExceeded));
}

#[test]
fn verify_rejects_mismatched_interlink_hash() {
    let mut block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    block.header.interlink_hash = Blake2bHash::from([1u8; Blake2bHash::SIZE]);
    block.header.n_bits = 0x1f010000u32.into();
    block.header.nonce = 31675;
    assert_eq!(block.verify(block.header.timestamp, NetworkId::Main), Err(BlockError::InterlinkHashMismatch));
}

#[test]
fn verify_rejects_mismatched_body_hash() {
    let mut block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    block.header.body_hash = Blake2bHash::from([1u8; Blake2bHash::SIZE]);
    block.header.n_bits = 0x1f010000u32.into();
    block.header.nonce = 41771;
    assert_eq!(block.verify(block.header.timestamp, NetworkId::Main), Err(BlockError::BodyHashMismatch));
}

#[test]
fn verify_rejects_invalid_body() {
    let mut block: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    {
        let body = block.body.as_mut().unwrap();
        body.transactions[0].validity_start_height = 5;
    }
    assert_eq!(block.verify(block.header.timestamp, NetworkId::Main), Err(BlockError::ExpiredTransaction));
}


#[test]
fn it_correctly_identifies_immediate_successors() {
    let block1: Block = Block::deserialize_from_vec(&hex::decode(BLOCK_169500).unwrap()).unwrap();
    let mut block2 = block1.clone();
    block2.header.height += 1;
    block2.header.prev_hash = block1.header.hash();
    block2.header.timestamp = block1.header.timestamp + 10;
    block2.interlink = block1.get_next_interlink(block2.header.n_bits.into());
    assert!(block2.is_immediate_successor_of(&block1));

    block2.header.height -= 2;
    assert!(!block2.is_immediate_successor_of(&block1));

    block2.header.height += 2;
    assert!(block2.is_immediate_successor_of(&block1));
    block2.interlink.hashes[0] = Blake2bHash::from([1u8; Blake2bHash::SIZE]);
    assert!(!block2.is_immediate_successor_of(&block1));
}


const TEST_BLOCK_LVL0: &str = "0001fafd3f1cc800309cb931081257b4e7edb0f9ea400187c550fd9141447ea086a56233a563441dad209cf3a31c37e185590734268721847871bfc089e77e9c5cd7477356533240dcbe035562dd48f8c1ffa4254ad2186fb994fdb5adc8ea75e4412a3e9c43ff3a5e14e0f7ab5d11e766caa58ee2d03f3a3aea30a42af8813d0a921e400000000000020000003c0002a8040470324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf011be440919634a6fe3ba5f8a7181fe4bb8212c13c00000100dbb5d09e18649a4bed123ce7e517d1207c6c794be21248e8461efd47393dd66c1be440919634a6fe3ba5f8a7181fe4bb8212c13c0000000002a04c690000000001502634000000010403003f4176e96ef20a71ad00207d7678ad02c3dff13656c345da0ce0712fa41140d629f61553aeab232c28acc214fd1648429a4bfb3385ae4052e870599434030000";
const TEST_BLOCK_LVL2: &str = "0001fafd3f1cc800309cb931081257b4e7edb0f9ea400187c550fd9141447ea086a56233a563441dad209cf3a31c37e185590734268721847871bfc089e77e9c5cd7477356533240dcbe035562dd48f8c1ffa4254ad2186fb994fdb5adc8ea75e4412a3e9c43ff3a5e14e0f7ab5d11e766caa58ee2d03f3a3aea30a42af8813d0a921e400000000000020000003c000aff4c0470324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf011be440919634a6fe3ba5f8a7181fe4bb8212c13c00000100dbb5d09e18649a4bed123ce7e517d1207c6c794be21248e8461efd47393dd66c1be440919634a6fe3ba5f8a7181fe4bb8212c13c0000000002a04c690000000001502634000000010403003f4176e96ef20a71ad00207d7678ad02c3dff13656c345da0ce0712fa41140d629f61553aeab232c28acc214fd1648429a4bfb3385ae4052e870599434030000";
const DUMMY_HASH: &str = "324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf";

#[test]
fn next_interlink_is_correct_1() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL0).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(4).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), dummy_hash.clone(), dummy_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_2() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL2).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(4).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), block_hash.clone(), block_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_3() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL0).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(2).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), block_hash.clone(), dummy_hash.clone(), dummy_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_4() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL2).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(2).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), block_hash.clone(), block_hash.clone(), block_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_5() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL0).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(8).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![dummy_hash.clone(), dummy_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_6() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL2).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(8).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), block_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_7() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL0).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(1).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), block_hash.clone(), block_hash.clone(), dummy_hash.clone(), dummy_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_8() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL2).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(1).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), block_hash.clone(), block_hash.clone(), block_hash.clone(), block_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_9() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL0).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(16).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![dummy_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}

#[test]
fn next_interlink_is_correct_10() {
    let block: Block = Block::deserialize_from_vec(&hex::decode(TEST_BLOCK_LVL2).unwrap()).unwrap();
    let interlink = block.get_next_interlink(Difficulty::from(16).into());

    let dummy_hash = Blake2bHash::from(DUMMY_HASH);
    let block_hash = block.header.hash::<Blake2bHash>();
    let expected = BlockInterlink::new(vec![block_hash.clone(), dummy_hash], &block_hash);
    assert_eq!(interlink, expected);
}
