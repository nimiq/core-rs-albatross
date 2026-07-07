//! Differential fuzz target proving the clamped Ed25519 secret scalar can be built with the stable
//! `Scalar::from_bytes_mod_order` (reduced) instead of the deprecated, feature-gated
//! `Scalar::from_bits` (unreduced) without changing any output.
//!
//! `nimiq_keys::PrivateKey::to_scalar` (and the multisig `ToScalar`) now use `from_bytes_mod_order`
//! so production no longer needs curve25519-dalek's `legacy_compatibility` feature. That is only
//! safe because the secret scalar is exclusively consumed as either a point multiplication against
//! an order-`l` base or a scalar multiplication by a reduced scalar — both of which are invariant
//! under reduction mod `l`. This target reconstructs those exact operations from arbitrary bytes
//! for both constructors and panics (an AFL crash) on any byte difference, so the fuzzer searches
//! for a counterexample.
//!
//! Input layout (needs >= 96 bytes): [0..32] secret-scalar seed (clamped), [32..64] a reduced
//! operand `h`, [64..96] a scalar `k` defining the order-`l` base `k·B`.
fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, scalar::Scalar};

        if data.len() < 96 {
            return;
        }
        let take = |r: std::ops::Range<usize>| -> [u8; 32] {
            let mut b = [0u8; 32];
            b.copy_from_slice(&data[r]);
            b
        };

        // Clamp the seed exactly as `PrivateKey::to_scalar` does (RFC 8032 secret_expand).
        let mut cb = take(0..32);
        cb[0] &= 248;
        cb[31] &= 127;
        cb[31] |= 64;

        #[allow(deprecated)]
        let x_bits = Scalar::from_bits(cb); // historical, unreduced
        let x_mod = Scalar::from_bytes_mod_order(cb); // new production, reduced

        let h = Scalar::from_bytes_mod_order(take(32..64));
        let k = Scalar::from_bytes_mod_order(take(64..96));
        // Order-`l` base, as produced by `mul_by_cofactor` on `B_v` or `k·B`.
        let base = &k * ED25519_BASEPOINT_TABLE;

        assert_eq!(
            (x_bits * base).compress(),
            (x_mod * base).compress(),
            "point mul differs for clamp {}",
            hex::encode(cb),
        );
        assert_eq!(
            (h * x_bits).as_bytes(),
            (h * x_mod).as_bytes(),
            "h·a differs for clamp {}",
            hex::encode(cb),
        );
        assert_eq!(
            (h * x_bits + k).to_bytes(),
            (h * x_mod + k).to_bytes(),
            "h·a + k differs for clamp {}",
            hex::encode(cb),
        );
    })
}
