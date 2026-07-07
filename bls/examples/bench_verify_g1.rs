//! Microbenchmark + equivalence check for the BLS signature verification
//! optimizations: the original two-pairing check, the single multi-pairing
//! rewrite (one final exponentiation instead of two) and the current
//! `PublicKey::verify_g1` (single multi-Miller loop with the Miller loop
//! coefficients of the fixed `g2` generator cached).
//!
//! Run with:
//!   cargo run --release --example bench_verify_g1 -p nimiq-bls
//!
//! The numbers are native, but the *ratio* between the methods carries over
//! to WASM, since they all do the same MNT6-753 field arithmetic.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use ark_ec::{pairing::Pairing, PrimeGroup};
use ark_ff::Zero;
use ark_mnt6_753::{G1Projective, G2Projective, MNT6_753};
use nimiq_bls::{KeyPair, SigHash, Signature};
use nimiq_hash::Hash;
use nimiq_test_utils::test_rng;
use nimiq_utils::key_rng::SecureGenerate;

/// The ORIGINAL implementation: two independent pairings (two final exps).
fn verify_old(hash_curve: G1Projective, pk: G2Projective, sig: G1Projective) -> bool {
    if pk.is_zero() {
        return false;
    }
    let lhs = MNT6_753::pairing(sig, G2Projective::generator());
    let rhs = MNT6_753::pairing(hash_curve, pk);
    lhs == rhs
}

/// The NEW implementation: single multi-pairing (one final exp).
/// `e(sig, g2) == e(H, pk)`  <=>  `e(sig, g2) * e(-H, pk) == 1`.
fn verify_new(hash_curve: G1Projective, pk: G2Projective, sig: G1Projective) -> bool {
    if pk.is_zero() {
        return false;
    }
    MNT6_753::multi_pairing([sig, -hash_curve], [G2Projective::generator(), pk]).is_zero()
}

/// Time `iters` calls of `f`, black-boxing inputs each call to defeat LICM.
fn time<F: FnMut() -> bool>(iters: u32, mut f: F) -> Duration {
    // Warmup.
    for _ in 0..3 {
        black_box(f());
    }
    let start = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    start.elapsed()
}

fn main() {
    let rng = &mut test_rng(false);

    // A real keypair + signature so both verifiers return `true`.
    let keypair = KeyPair::generate(rng);
    let message = "benchmark message".to_string();
    let hash: SigHash = message.hash();
    let hash_curve = Signature::hash_to_g1(hash);
    let pk = keypair.public_key.public_key;
    let sig = keypair.sign(&message).signature;

    // ---- Equivalence check (the part that matters for consensus) ----
    // Valid signature: both must accept, and must agree with the real impl.
    assert!(verify_old(hash_curve, pk, sig));
    assert!(verify_new(hash_curve, pk, sig));
    assert!(keypair
        .public_key
        .verify_g1(hash_curve, &keypair.sign(&message)));

    // Tampered signature: both must reject.
    let bad_sig = keypair.sign(&"other message".to_string()).signature;
    assert_eq!(
        verify_old(hash_curve, pk, bad_sig),
        verify_new(hash_curve, pk, bad_sig)
    );
    assert!(!verify_new(hash_curve, pk, bad_sig));

    // Point-at-infinity pk: both must reject (guard preserved).
    assert!(!verify_old(hash_curve, G2Projective::zero(), sig));
    assert!(!verify_new(hash_curve, G2Projective::zero(), sig));

    // Randomized equivalence: many random (pk, sig, msg) combos must agree.
    for i in 0..200 {
        let kp = KeyPair::generate(rng);
        let msg = format!("msg {i}");
        let h: SigHash = msg.hash();
        let hc = Signature::hash_to_g1(h);
        let p = kp.public_key.public_key;
        // Mix valid and invalid: half correctly signed, half cross-signed.
        let s = if i % 2 == 0 {
            kp.sign(&msg).signature
        } else {
            keypair.sign(&msg).signature
        };
        assert_eq!(
            verify_old(hc, p, s),
            verify_new(hc, p, s),
            "mismatch at i={i}"
        );
    }
    println!("equivalence: OK (valid, tampered, point-at-infinity, 200 random cases)\n");

    // ---- Timing: the pairing verification ----
    let verify_iters = 200u32;

    let signature = keypair.sign(&message);
    let t_old = time(verify_iters, || verify_old(hash_curve, pk, sig));
    let t_new = time(verify_iters, || verify_new(hash_curve, pk, sig));
    let t_cur = time(verify_iters, || {
        keypair.public_key.verify_g1(hash_curve, &signature)
    });

    let old_ms = t_old.as_secs_f64() * 1e3 / verify_iters as f64;
    let new_ms = t_new.as_secs_f64() * 1e3 / verify_iters as f64;
    let cur_ms = t_cur.as_secs_f64() * 1e3 / verify_iters as f64;

    println!("verify (pairing check), {verify_iters} iters each:");
    println!("  old (2 pairings):                  {old_ms:8.3} ms/op");
    println!(
        "  new (1 multipairing):              {new_ms:8.3} ms/op  ({:.2}x vs old)",
        old_ms / new_ms
    );
    println!(
        "  current (cached g2 prep, verify_g1): {cur_ms:6.3} ms/op  ({:.2}x vs old, {:.2}x vs multipairing)\n",
        old_ms / cur_ms,
        new_ms / cur_ms
    );

    // ---- Timing: aggregation of ~342 signer slots (unchanged by Option 1) ----
    // Build ~32 distinct validator keys; aggregate TWO_F_PLUS_ONE = 342 slots.
    const SIGNER_SLOTS: usize = 342;
    let val_keys: Vec<G2Projective> = (0..32)
        .map(|_| KeyPair::generate(rng).public_key.public_key)
        .collect();

    let agg_iters = 300u32;
    let t_agg = time(agg_iters, || {
        let mut acc = G2Projective::zero();
        for i in 0..SIGNER_SLOTS {
            acc += val_keys[i % val_keys.len()];
        }
        let _ = black_box(acc);
        true
    });
    let agg_ms = t_agg.as_secs_f64() * 1e3 / agg_iters as f64;
    println!("aggregate {SIGNER_SLOTS} signer slots (G2 adds, unchanged): {agg_ms:8.3} ms/op\n");

    // ---- Per-election-block extrapolation ----
    let block_old = agg_ms + old_ms;
    let block_new = agg_ms + new_ms;
    let block_cur = agg_ms + cur_ms;
    println!("per election block (aggregation + verification), native:");
    println!("  old:     {block_old:8.3} ms");
    println!("  new:     {block_new:8.3} ms");
    println!("  current: {block_cur:8.3} ms");
    println!(
        "  whole-block speedup (old -> current): {:.2}x ({:.1}% faster)",
        block_old / block_cur,
        (1.0 - block_cur / block_old) * 100.0
    );
}
