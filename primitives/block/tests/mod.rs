use std::str::FromStr;

use beserial::{Deserialize, Serialize};
use nimiq_block::{IndividualSignature, MacroBlock, MacroBody, MacroHeader, MultiSignature};
use nimiq_bls::{CompressedPublicKey, KeyPair};
use nimiq_collections::bitset::BitSet;
use nimiq_handel::update::LevelUpdate;
use nimiq_hash::{Blake2bHasher, Hasher};
use nimiq_keys::{Address, PublicKey};
use nimiq_primitives::slots::ValidatorsBuilder;
use nimiq_vrf::VrfSeed;

#[test]
fn it_can_convert_macro_block_into_slots() {
    let hash = Blake2bHasher::default().digest(&[]);

    let slot_allocation = vec![
        (
            [0u8; 20],
            127u16,
            "04c998c7de9f44ddfcf41f2359edd1145926a6277bbf38ce72bcae395a0afc10",
            "0127c89b5812d724944cc0ce922487c99768398dc1772bfcff80989546ea199254a1945ac705f1fca2cc3cf0ce64737672a63002887530fef3646a3e5135806f9d400597fbec1a2045902cfdd7c4272f2acf296338ddcd9353b16736f5284f",
        ),
        (
            [1u8; 20],
            129u16,
            "04c998c7de9f44ddfcf41f2359edd1145926a6277bbf38ce72bcae395a0afc10",
            "010ab37c2de8dc0b1c157aa312c8dc6575e7a2f2c0de346ae2a1d9ea7bc13a7d7a5f403ee482403965db23e86c2e9e3ceed7de3396874b8c2b002b1e8333492d28d63b3fa786c4002083d1a7ba35f7a4d9cc0d14fcad895a4503a926dfda51",
        ),
        (
            [2u8; 20],
            126u16,
            "e1c054925c1ee292da9b41735d14a0bb31f3abf722277859a67d9039f8b635c3",
            "8176aef893475e9fea3d87a0abd092cad3ed59a48daa243c8b5e2a4a81c056ae6962973928a3baa617d7b150b11c4c3c124e8e17b74e057a7c6c364f99c53d5b5366e847747de72a3f1a24b62a51febfe4f99a19ac085d3c711210e5027679",
        ),
        (
            [3u8; 20],
            130u16,
            "c52db7a3414e84aec2b8b662b104eed7a2027677e20412534dba4cbb121263e5",
            "809dde62e40fa1d9d37309a35635070d249eceb9db04ef511d6c0f1f950c3ddf3ba64303c1e8844714cd2bb68f8968bb1de7c0f26c542c9db74778287e943fc120fc1c848fcd89cf56a918ae1fc5c6d4a1b8f4a1023679eed4658d50533348",
        ),
    ];

    let mut builder = ValidatorsBuilder::new();

    for (validator_address, num_slots, signing_key, voting_key) in slot_allocation {
        let signing_key = PublicKey::from_str(signing_key).unwrap();
        let voting_key = CompressedPublicKey::from_str(voting_key).unwrap();
        let address = Address::from(validator_address);

        for _ in 0..num_slots {
            builder.push(address.clone(), voting_key.clone(), signing_key);
        }
    }

    let validator_slots = builder.build();

    let macro_block = MacroBlock {
        header: MacroHeader {
            version: 1,
            block_number: 42,
            view_number: 0,
            timestamp: 0,
            parent_hash: hash.clone(),
            parent_election_hash: hash.clone(),
            seed: VrfSeed::default(),
            extra_data: vec![],
            state_root: hash.clone(),
            body_root: hash,
            history_root: [0u8; 32].into(),
        },
        justification: None,
        body: Some(MacroBody {
            validators: Some(validator_slots.clone()),
            pk_tree_root: None,
            lost_reward_set: BitSet::new(),
            disabled_set: BitSet::new(),
        }),
    };

    let validators_from_macro = macro_block.get_validators().unwrap();

    assert_eq!(validator_slots, validators_from_macro);
}

fn create_multisig() -> MultiSignature {
    let raw_key = hex::decode(
        "1b9e470e0deb06fe55774bb2cf499b411f55265c10d8d78742078381803451e058c88\
        391431799462edde4c7872649964137d8e03cd618dd4a25690c56ffd7f42fb7ae8049d29f38d569598b38d4\
        39f69107cc0b6f4ecd00a250c74409510100",
    )
    .unwrap();
    let key_pair = KeyPair::deserialize_from_vec(&raw_key).unwrap();
    let signature = key_pair.sign(&"foobar");
    IndividualSignature::new(signature, 1).as_multisig()
}

#[test]
fn test_serialize_deserialize_level_update() {
    let update = LevelUpdate::new(create_multisig(), None, 2, 3);
    let data = update.serialize_to_vec();
    let update_2: LevelUpdate<MultiSignature> = Deserialize::deserialize_from_vec(&data).unwrap();

    assert_eq!(data.len(), update.serialized_size());
    assert_eq!(update.serialized_size(), 298);
    // assert!(update_2.individual.is_none()); // not publicly accessible
    assert_eq!(update_2.level(), 2);
    assert_eq!(update_2.origin(), 3);
}

#[test]
fn test_serialize_deserialize_with_message() {
    let update = LevelUpdate::new(create_multisig(), None, 2, 3).with_tag(42u64);
    assert_eq!(update.serialized_size(), 298 + 8);
}
