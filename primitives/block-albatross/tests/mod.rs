use std::convert::TryInto;
use std::str::FromStr;

use beserial::Deserialize;
use nimiq_block_albatross::{MacroBlock, MacroExtrinsics, MacroHeader};
use nimiq_bls::{CompressedPublicKey, Signature};
use nimiq_collections::bitset::BitSet;
use nimiq_hash::{Blake2bHasher, Hasher};
use nimiq_keys::Address;
use nimiq_primitives::slot::{Slots, ValidatorSlotBand, ValidatorSlots};

#[test]
fn it_can_convert_macro_block_into_slots() {
    let hash = Blake2bHasher::default().digest(&vec![]);

    let signature_bytes = hex::decode(
        "8001733b6e6edf3e1c6feb8841d32abe26d05163fddf6d2\
    2179b17c7b1ead82e8fcad81e9685da8102ae3ae8c3b80d098545cadfba0d5310b1aa48f97ee649ec58943ae68d68a9\
    f8e9fff2830da42e18e6f9a58781f3f8757795605bcf577588",
    )
    .unwrap();
    let signature = Signature::deserialize_from_vec(&signature_bytes).unwrap();

    let slot_allocation = vec![
        (
            127u16,
            "0001bf7a0533de80527693870f5b92c188c592fb83c4b31df37e036e2f858812e2fb0b74f0810537a\
        1c8bf9141055e31c620f8c1bdebe78c8d427dec5e80b068b56a5c6a404766b31f937e55afc9801d6b163f2662b4\
        60cc9e19a650939f6ad10001856f0a4907a867f99e117e3d800b7cbad0a217ccc9c9f7559a4291f934636080636\
        a03714a485d6512680078e99dd1aa70e334afdd513d56f2ebd08cc9cda0fa17f425c957698734013aa4d83dd947\
        3fc55eb75e6d80c8532c41acbe0de000007e8ba0474c0df989bf58a693b5e28fa2a9f1497c164f6c6d95bf24186\
        d00f94b6166fabaecf501a84e1dfd54d46bd9c904d8e57914052491c7710f3dfc7f9f9b0c0db9a3b655b672c238\
        00498569476df4805905bfce4c8da83efdf9315e0fc8aa9e5e5bed39a9811082e0d775f996bb9e56",
        ),
        (
            129u16,
            "80011040dc327cfe4bd325a4d16247f272e8946c5322bd908314a7045e5ab38185e33b41cf445575c\
        eee1d01f32a6aee0133bd59ca4aa2fb893f6bb087019d11cd5888471bc265939a785251c962ffa9591e5878bf64\
        368b19a6894c24e6b7a60001256f8a0e583ae90d37ec68a6eb6b8c46df22b4acf59efb8af4ec67c84593c88f3bd\
        5bc52683725da7c2bd3ddf72e30094683e60aae28077d3f90216f03b0debf4998846eebf4c96f36926f1f0b7a6a\
        ddf24ee94b7a8a8ed2f0d9d44fec380000968315715c8bb78e3485c7a5bed30e6234d0d95aab7a1208fa8303e0f\
        684c0bc6841cee6f4363272d882410236e3366a5d32be26bed586c64da6e067a67b3c851d40a1e68d4b52c701ee\
        29b97b13d78d2c8c318fa88ec345da01cc4fae9da29a95eca005ae9ea7698cba08fc813d4d71efbe",
        ),
        (
            126u16,
            "80017935c655463970607d01d8758904c92ce53abefaa1b83256d8a1ddd7d13cd593528085478aa76\
        e7bd8328d1620b7b1b12ef02cddf7cee4ede9558a6da53d2612d4c0cc66274fd1c5e3faa0d9dc5da00db5eac9a8\
        4b6e549609b019c8dae400005186dbf7c31b58945b8cff8cff3617764437c94ff5d8175b2502132ab79c7b4ab65\
        85476a8f993dda17a80c3f20dc476a59c3fa8462cddd5d91b118d299ccac6d50c69ca04defe66a6ccf65ebee599\
        cdd4ef2d0830503b095782727e615a00014249cb77c70d6ce51ea1cb5f662327d6b37286f1753e15489c10665fd\
        102c1620a9317a99a74b729e465a0a5f0e851f3d6c03580c2fe543813f464319ce33f389cdce47c2a22a75debcd\
        5760d8916ef490edbfa32b9bfa6a77d20d4338266a15f2277cce1bde7e265c84d2a727653b31884c",
        ),
        (
            130u16,
            "000051d6eacd61eda8723e91968cd5036162b83268f2613129ca4c9cf79c130fff93892399bff6c08\
        a7990b14cc96ad155282f3d6503c25fe439783ff65f579448b2b5b1574bf223004511d43ccb1461ee083783171f\
        6e15dca59ee7b7c939fd0000a9cba7a0d407c3deed22b6b7d28f2623a08f747d59a2fca5f2f93785d1f1e06c952\
        b47d7a182ee43b4368b5c6fe8e71b3d2678fd7f3171124d6fd39ee99969f9f30c7763167411819246601f5cdf71\
        dc2ae4291faae780ee1ca06101574900001265ce5df8a7d7d19c2ac1b047e828f732a471a3363b43899d7208b5b\
        cb8d5769479ec5a25e1e4e239e38b5a3adf1d14c756618185971cae9001d1e87bfed3b87ba3acc6e5b7e87bd7bd\
        5fe88760dedd7fab48fcb197725114208510ecaa31020442803a81db35ac5f67f29d87924a0eaf76",
        ),
    ];
    let validator_slots: ValidatorSlots = slot_allocation
        .into_iter()
        .map(|(num_slots, data)| {
            let pubkey = CompressedPublicKey::from_str(&data[..576]).unwrap();
            let address = Address::from_any_str(&data[576..]).unwrap();
            ValidatorSlotBand::new(pubkey, address, num_slots)
        })
        .collect();

    let macro_block = MacroBlock {
        header: MacroHeader {
            version: 1,
            validators: validator_slots.clone(),
            block_number: 42,
            view_number: 0,
            parent_macro_hash: hash.clone(),
            seed: signature.compress().into(),
            parent_hash: hash.clone(),
            state_root: hash.clone(),
            extrinsics_root: hash.clone(),
            transactions_root: [0u8; 32].into(),
            timestamp: 0,
        },
        justification: None,
        extrinsics: Some(MacroExtrinsics {
            slashed_set: BitSet::new(),
        }),
    };

    let slots = Slots::new(validator_slots);
    let slots_from_macro: Slots = macro_block.try_into().unwrap();

    assert_eq!(slots, slots_from_macro);
}
