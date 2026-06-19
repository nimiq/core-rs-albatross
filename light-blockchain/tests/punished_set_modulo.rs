use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_collections::BitSet;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_vrf::VrfEntropy;

/// Builds a punished set that covers every valid slot `0..SLOTS` plus one out-of-range bit
/// (index `SLOTS`), giving a popcount of `SLOTS + 1`. This evades the `len() == SLOTS`
/// all-disabled fast path in `compute_slot_number` while still disabling every in-range slot.
fn over_covering_punished_set() -> BitSet {
    let mut bs = BitSet::new();
    for slot in 0..Policy::SLOTS as usize {
        bs.insert(slot);
    }
    bs.insert(Policy::SLOTS as usize);
    bs
}

/// Regression test for the punished-set modulo-by-zero DoS. A peer-supplied election header
/// carrying an over-covering punished set must not be able to crash `compute_slot_number`, the
/// proposer-selection step reached from `LightBlockchain::get_proposer` on light/pico clients.
#[test]
fn forged_punished_set_does_not_panic_compute_slot_number() {
    let bs = over_covering_punished_set();

    // The over-covering set survives binary (de)serialization unchanged: the wire format has no
    // deserialize-time index/count bound, so this is exactly what a light/pico client would
    // adopt from a malicious peer
    let wire = bs.serialize_to_vec();
    let bs = BitSet::deserialize_from_vec(&wire).expect("bitset deserializes from wire");
    assert_eq!(bs.len(), Policy::SLOTS as usize + 1);

    // Before the fix `slots` was empty and `offset % slots.len()` panicked with a division by
    // zero. After the fix an empty viable set falls back to accepting any slot
    for offset in [0, 1, 7, 511, 512, u32::MAX] {
        let slot = <LightBlockchain as AbstractBlockchain>::compute_slot_number(
            offset,
            VrfEntropy::default(),
            bs.clone(),
        );
        assert!(
            slot < Policy::SLOTS,
            "expected a valid slot index, got {slot}"
        );
    }
}
