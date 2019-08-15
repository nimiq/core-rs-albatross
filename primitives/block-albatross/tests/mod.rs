use std::convert::{TryFrom, TryInto};
use std::iter::repeat;

use beserial::Deserialize;
use nimiq_block_albatross::{MacroBlock, MacroHeader, MacroExtrinsics, SlotAddresses};
use nimiq_bls::bls12_381::Signature;
use nimiq_bls::bls12_381::lazy::LazyPublicKey;
use nimiq_collections::compressed_list::CompressedList;
use nimiq_hash::{Blake2bHasher, Hasher};
use nimiq_keys::Address;
use nimiq_primitives::validators::Slots;
use nimiq_primitives::coin::Coin;

#[test]
fn it_can_convert_macro_block_into_slots() {
    let hash = Blake2bHasher::default().digest(&vec![]);

    let signature_bytes = hex::decode("b9674ac1bbb4770ad291acc2b860e9120c609893a3840fbfdf2946911f00255f8995974c9b0ef3835ab4442ccbac9739").unwrap();
    let signature = Signature::deserialize_from_vec(&signature_bytes).unwrap();

    let slot_allocation = vec![
        (127u16, "828aa810f80b9e200bb3310a3f837a8b1642e14440ec65ba8eaf801b9e1b81e69adc706b1ba6ed844cc793621dbd5e220ab235eac6d2b03c14e7a4da200759fee5f15903b9ef07602f7d50346fb25202f399affec1878cbfaa64cccdf0054cc6"),
        (129u16, "8338ee9e2ff9a21e07ecccf5fb2bc184db64b560389b6ef1ef215e3747b1934e40337a18baa012b11181944764cfd87104d83eb43629adaee0ac158552edc4fe2a171d6bdd2144b97aa845511fd4c12608bba777546ffeaf7782885d281d4e43"),
        (126u16, "accc156ac10d2d1cc7fc0c565acea9295e2d258608f280c076b4679c5a465fb9fcd8f22c6f9179cd8f7d63aaa04b9d3a088b1f3764cb93c67dc3a21c94666f5b729fa9f058ad65eb023aeaaaa2c39112bac4c613374d82a0e3407df4595d1535"),
        (130u16, "abdaf5ac13036550362c2d3c5f1848fd6ab1898c75311381bd022d6a2a7909d526ad7a6aaafbaf8f64f11a3af5f220fa0a150b022394ff5da765016b7e6a2525fbe63c65b2e382989de3ecb04038e24c9f782e7965c2b3ec179c7715ecf7f191"),
    ];
    let validators: CompressedList<LazyPublicKey> = slot_allocation.iter()
        .map(|(num, entry)| {
            let vec = hex::decode(entry).unwrap();
            let key = LazyPublicKey::deserialize(&mut &vec[..]).unwrap();
            (num, key)
        })
        .flat_map(|(num, key)|
            repeat(key)
            .take((*num) as usize)
        )
        .collect();

    let address_allocation = vec![
        (126u16, "0fc8aa9e5e5bed39a9811082e0d775f996bb9e56"),
        (127u16, "a29a95eca005ae9ea7698cba08fc813d4d71efbe"),
        (129u16, "6a15f2277cce1bde7e265c84d2a727653b31884c"),
        (130u16, "31020442803a81db35ac5f67f29d87924a0eaf76"),
    ];
    let slot_addresses: CompressedList<SlotAddresses> = address_allocation.iter()
        .flat_map(|(num, address)|
            repeat(SlotAddresses {
                reward_address: Address::from(*address),
                staker_address: Address::from(*address),
            })
            .take((*num) as usize)
        )
        .collect();


    let macro_block = MacroBlock {
        header: MacroHeader {
            version: 1,
            validators: validators.clone(),
            block_number: 42,
            view_number: 0,
            parent_macro_hash: hash.clone(),
            seed: signature.compress(),
            parent_hash: hash.clone(),
            state_root: hash.clone(),
            extrinsics_root: hash.clone(),
            transactions_root: [0u8; 32].into(),
            timestamp: 0,
        },
        justification: None,
        extrinsics: Some(MacroExtrinsics {
            slot_addresses: slot_addresses.clone(),
            slash_fine: Coin::try_from(8u64).unwrap(),
        }),
    };

    let slots: Slots = macro_block.try_into().unwrap();

    validators.into_iter()
        .zip(slot_addresses.into_iter())
        .zip(slots.into_iter())
        .for_each(|((key, addresses), slot)| {
            assert_eq!(&slot.public_key, key);
            assert_eq!(&addresses.staker_address, &slot.staker_address);
            assert_eq!(&addresses.reward_address, slot.reward_address());
        });
}
