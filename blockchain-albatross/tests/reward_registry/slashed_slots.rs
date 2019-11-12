use nimiq_blockchain_albatross::reward_registry::SlashedSlots;
use nimiq_bls::bls12_381::KeyPair;
use nimiq_bls::SecureGenerate;
use nimiq_collections::bitset::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::validators::{Slot, Slots};

#[test]
fn it_can_iterate() {
    let bls_keypair = KeyPair::generate(&mut rand::thread_rng());
    let slot_1 = Slot {
        public_key: bls_keypair.public.clone().into(),
        reward_address_opt: None,
        staker_address: Address::from([3u8; 20]),
    };
    let slot_2 = Slot {
        public_key: bls_keypair.public.clone().into(),
        reward_address_opt: None,
        staker_address: Address::from([5u8; 20]),
    };

    let mut slots_vec = Vec::<Slot>::with_capacity(96);
    for _ in 0..32 {
        slots_vec.push(slot_1.clone());
    }
    for _ in 32..96 {
        slots_vec.push(slot_2.clone());
    }

    let slots = Slots::new(slots_vec, Coin::from_u64_unchecked(42));

    let mut slashed_set = BitSet::new();
    slashed_set.insert(31);

    let slashed_slots = SlashedSlots::new(&slots, &slashed_set);

    // Iterate slot states
    let mut slot_states = slashed_slots.slot_states();
    for _ in 0..31 {
        let next = slot_states.next().unwrap();
        assert_eq!(next, (&slot_1, true));
    }
    let next = slot_states.next().unwrap();
    assert_eq!(next, (&slot_1, false)); // Slashed
    for _ in 32..96 {
        let next = slot_states.next().unwrap();
        assert_eq!(next, (&slot_2, true));
    }
    assert_eq!(slot_states.next(), None);

    // Iterate enabled slots
    let mut enabled = slashed_slots.enabled();
    for _ in 0..31 {
        let next = enabled.next().unwrap();
        assert_eq!(next, &slot_1);
    }
    for _ in 32..96 {
        let next = enabled.next().unwrap();
        assert_eq!(next, &slot_2);
    }
    assert_eq!(enabled.next(), None);

    // Iterate slashed slots
    let mut slashed = slashed_slots.slashed();
    assert_eq!(slashed.next(), Some(&slot_1));
    assert_eq!(slashed.next(), None);
}
