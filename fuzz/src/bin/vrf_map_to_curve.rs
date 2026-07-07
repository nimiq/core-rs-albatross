//! Differential fuzz target for the VXEdDSA map-to-curve step.
//!
//! `nimiq_vrf::map_to_curve_compressed` is a reimplementation of curve25519-dalek 4.x's
//! `EdwardsPoint::nonspec_map_to_curve::<Sha512>`, which was removed in 5.0. That point is folded
//! into every VRF seed in the chain history, so the reimplementation MUST match the old one
//! byte-for-byte. This target feeds arbitrary bytes to both and panics (an AFL crash) on any
//! mismatch, so the fuzzer searches for a counterexample.
fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use curve25519_dalek_4x::edwards::EdwardsPoint as EdwardsPoint4x;
        use sha2_0_10::Sha512 as Sha512_4x;

        // Reference: the exact function that shipped in curve25519-dalek 4.x.
        #[allow(deprecated)]
        let reference = EdwardsPoint4x::nonspec_map_to_curve::<Sha512_4x>(data)
            .compress()
            .to_bytes();

        // Ported: the implementation now used in production by nimiq-vrf.
        let ported = nimiq_vrf::vrf::map_to_curve_compressed(data);

        assert_eq!(
            ported,
            reference,
            "map-to-curve mismatch on input {}",
            hex::encode(data),
        );
    })
}
