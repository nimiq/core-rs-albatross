//! Differential test: building the clamped secret scalar with the stable `from_bytes_mod_order`
//! (reduced) must match the deprecated, feature-gated `from_bits` (unreduced) that production used
//! before. Consensus-critical (the scalar derives every signature); safe because the scalar is only
//! ever multiplied by an order-`l` point or a reduced scalar, both of which reduce it mod `l`.
//!
//! Fixed edge cases run by default (fast per-PR guard); exhaustive fuzzing is the
//! `keys_scalar_reduction` AFL target. Set `KEYS_FUZZ_ITERS`/`KEYS_FUZZ_SEED` for a random sample.
//! The reference `from_bits` needs `legacy_compatibility`, enabled only for the test build.

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
    // Fixed cases suffice by construction: the two constructors can only differ by a multiple of
    // `l`, and a clamped scalar's top byte spans 0x40..0x7f, so these patterns cover every possible
    // number of `l`-subtractions.
    let mut fixed: Vec<[u8; 32]> = [
        0x00u8, 0xff, 0x55, 0xaa, 0x0f, 0xf0, 0x33, 0xcc, 0x01, 0x80, 0x3f, 0x7f,
    ]
    .into_iter()
    .map(|b| clamp([b; 32]))
    .collect();
    // An incrementing pattern, for a non-uniform value.
    let mut inc = [0u8; 32];
    for (i, b) in inc.iter_mut().enumerate() {
        *b = i as u8;
    }
    fixed.push(clamp(inc));

    // Optional random sample on top of the fixed cases, off by default.
    let seed = std::env::var("KEYS_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let iters = std::env::var("KEYS_FUZZ_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let mut rng = StdRng::seed_from_u64(seed);

    for cb in fixed {
        check_one(cb, &mut rng).unwrap();
    }

    // Random clamped seeds (only valid secret-scalar inputs, as production feeds).
    for _ in 0..iters {
        let mut hash_low = [0u8; 32];
        rng.fill_bytes(&mut hash_low);
        check_one(clamp(hash_low), &mut rng).unwrap();
    }
}
