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

    let signature_bytes = hex::decode("b9674ac1bbb4770ad291acc2b860e9120c609893a3840fbfdf2946911f00255f8995974c9b0ef3835ab4442ccbac9739").unwrap();
    let signature = Signature::deserialize_from_vec(&signature_bytes).unwrap();

    let slot_allocation = vec![
        (127u16, "828aa810f80b9e200bb3310a3f837a8b1642e14440ec65ba8eaf801b9e1b81e69adc706b1ba6ed844cc793621dbd5e220ab235eac6d2b03c14e7a4da200759fee5f15903b9ef07602f7d50346fb25202f399affec1878cbfaa64cccdf0054cc60fc8aa9e5e5bed39a9811082e0d775f996bb9e56"),
        (129u16, "8338ee9e2ff9a21e07ecccf5fb2bc184db64b560389b6ef1ef215e3747b1934e40337a18baa012b11181944764cfd87104d83eb43629adaee0ac158552edc4fe2a171d6bdd2144b97aa845511fd4c12608bba777546ffeaf7782885d281d4e43a29a95eca005ae9ea7698cba08fc813d4d71efbe"),
        (126u16, "accc156ac10d2d1cc7fc0c565acea9295e2d258608f280c076b4679c5a465fb9fcd8f22c6f9179cd8f7d63aaa04b9d3a088b1f3764cb93c67dc3a21c94666f5b729fa9f058ad65eb023aeaaaa2c39112bac4c613374d82a0e3407df4595d15356a15f2277cce1bde7e265c84d2a727653b31884c"),
        (130u16, "abdaf5ac13036550362c2d3c5f1848fd6ab1898c75311381bd022d6a2a7909d526ad7a6aaafbaf8f64f11a3af5f220fa0a150b022394ff5da765016b7e6a2525fbe63c65b2e382989de3ecb04038e24c9f782e7965c2b3ec179c7715ecf7f19131020442803a81db35ac5f67f29d87924a0eaf76"),
    ];
    let validator_slots: ValidatorSlots = slot_allocation
        .into_iter()
        .map(|(num_slots, data)| {
            let pubkey = CompressedPublicKey::from_str(&data[..192]).unwrap();
            let address = Address::from_any_str(&data[192..]).unwrap();
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
