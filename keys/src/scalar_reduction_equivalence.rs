//! Differential test proving that the clamped Ed25519 secret scalar can be built with the stable,
//! non-deprecated [`Scalar::from_bytes_mod_order`] instead of the deprecated, feature-gated
//! [`Scalar::from_bits`] WITHOUT changing any observable output.
//!
//! Background: `PrivateKey::to_scalar` (and the multisig `ToScalar`) historically used
//! `from_bits`, which stores the clamped scalar UNREDUCED (`a`, not `a mod l`). `from_bits` is only
//! available with curve25519-dalek's `legacy_compatibility` feature, which may be dropped upstream
//! (exactly as `nonspec_map_to_curve` was removed in 5.0). To avoid depending on it, production now
//! uses `from_bytes_mod_order`, which stores `a mod l`.
//!
//! This is consensus-critical: those scalars derive every signature and validator key. Reduction is
//! only safe because the secret scalar is exclusively consumed either as a point multiplication
//! against an order-`l` base (`a·B = (a mod l)·B`) or as a scalar multiplication by an
//! already-reduced scalar (dalek's Montgomery multiply reduces `a` mod `l` before it is ever used or
//! serialized). This module asserts that equivalence directly, over a large fuzzed input space, so
//! the switch is proven rather than merely argued. The reference `from_bits` path requires
//! `legacy_compatibility`, which is enabled for the test build via a dev-dependency; production no
//! longer enables it.
//!
//! The seed and iteration count can be overridden with `KEYS_FUZZ_SEED` / `KEYS_FUZZ_ITERS`.

use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, edwards::EdwardsPoint, scalar::Scalar};
use rand::{rngs::StdRng, Rng, SeedableRng};

/// The exact RFC 8032 clamp used by `PrivateKey::to_scalar`: the low 32 bytes of a digest with the
/// standard Ed25519 bit fixing.
fn clamp(seed_hash_low: [u8; 32]) -> [u8; 32] {
    let mut b = seed_hash_low;
    b[0] &= 248;
    b[31] &= 127;
    b[31] |= 64;
    b
}

/// A reduced scalar from 64 uniform bytes (models `h`, `s`, `c`, and nonce scalars, which are all
/// produced by `from_hash` / `from_bytes_mod_order[_wide]` in production).
fn reduced(rng: &mut StdRng) -> Scalar {
    let mut wide = [0u8; 64];
    rng.fill_bytes(&mut wide);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// An order-`l` point (as produced by `mul_by_cofactor` on `B_v`, or `k·basepoint`). Production
/// never multiplies the secret scalar by a point outside the prime-order subgroup.
fn order_l_point(rng: &mut StdRng) -> EdwardsPoint {
    &reduced(rng) * ED25519_BASEPOINT_TABLE
}

/// Runs the equivalence checks for a single clamped seed against a batch of random operands.
/// Returns the offending clamp bytes on the first mismatch.
fn check_one(cb: [u8; 32], rng: &mut StdRng) -> Result<(), String> {
    #[allow(deprecated)]
    let x_bits = Scalar::from_bits(cb); // historical, unreduced
    let x_mod = Scalar::from_bytes_mod_order(cb); // new production, reduced

    // (1) Point multiplication against order-`l` bases: the basepoint and random `k·B` points.
    for base in [&Scalar::ONE * ED25519_BASEPOINT_TABLE, order_l_point(rng)] {
        if (x_bits * base).compress() != (x_mod * base).compress() {
            return Err(format!("point mul differs for clamp {}", hex::encode(cb)));
        }
    }

    // (2) Scalar multiplication by a reduced scalar, plus the downstream add + serialize that
    // production performs. Models VRF `s = r + h·a` and multisig `c·(s·sk) + secret`.
    for _ in 0..8 {
        let h = reduced(rng);
        let r = reduced(rng);
        let c = reduced(rng);
        let s = reduced(rng);
        let secret = reduced(rng);

        if (h * x_bits).as_bytes() != (h * x_mod).as_bytes() {
            return Err(format!("h·a differs for clamp {}", hex::encode(cb)));
        }
        if (r + h * x_bits).to_bytes() != (r + h * x_mod).to_bytes() {
            return Err(format!("r + h·a differs for clamp {}", hex::encode(cb)));
        }
        if (c * (s * x_bits) + secret).as_bytes() != (c * (s * x_mod) + secret).as_bytes() {
            return Err(format!(
                "c·(s·sk) + secret differs for clamp {}",
                hex::encode(cb)
            ));
        }
    }

    Ok(())
}

#[test]
fn from_bytes_mod_order_matches_from_bits() {
    // Fixed edge cases: the extreme clamped values and a few structured ones.
    let fixed: [[u8; 32]; 4] = [
        clamp([0x00; 32]),
        clamp([0xff; 32]),
        clamp([0x55; 32]),
        clamp([0xaa; 32]),
    ];

    let seed = std::env::var("KEYS_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let iters = std::env::var("KEYS_FUZZ_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000u64);
    let mut rng = StdRng::seed_from_u64(seed);

    for cb in fixed {
        check_one(cb, &mut rng).unwrap();
    }

    // Random clamped seeds. Clamp is applied so we only ever exercise valid secret-scalar inputs,
    // which is what production feeds these constructors.
    for _ in 0..iters {
        let mut hash_low = [0u8; 32];
        rng.fill_bytes(&mut hash_low);
        check_one(clamp(hash_low), &mut rng).unwrap();
    }
}
