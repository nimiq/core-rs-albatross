//! Faithful red/green test for the unbounded allocation in `Validators::hash()`
//! reached via `Block::hash_cached()` on a RAW (unverified) `RequestBlock`
//! response, exactly as `consensus::sync::request_block` does.

use nimiq_block::{Block, MacroBlock, MacroHeader};
use nimiq_bls::{CompressedPublicKey, LazyPublicKey};
use nimiq_keys::{Address, Ed25519PublicKey};
use nimiq_primitives::slots_allocation::{Validator, Validators};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;

fn craft_validator(seed: u8, slots: std::ops::Range<u16>) -> Validator {
    // 285 arbitrary bytes; no curve validation is performed on this path because
    // `Validators::hash()` uses the raw compressed bytes via `.compressed().as_ref()`.
    let raw = [seed; CompressedPublicKey::SIZE];
    let compressed = CompressedPublicKey { public_key: raw };
    let voting_key = LazyPublicKey::from_compressed(&compressed);
    let signing_key = Ed25519PublicKey::from_bytes(&[seed.wrapping_add(1); 32])
        .expect("32-byte ed25519 verification key bytes");
    let address = Address::from([seed; 20]);
    Validator::new(address, voting_key, signing_key, slots)
}

#[test]
fn unbounded_alloc_via_hash_cached_on_raw_response() {
    // 285 * 65535 ~= 18.68 MB per validator. 200 validators -> ~3.7 GB.
    let n_validators: u16 = 200;
    let k: u16 = 65535; // num_slots per validator = 65535

    let mut validators = Vec::new();
    for i in 0..n_validators {
        validators.push(craft_validator((i % 251 + 1) as u8, 0..k));
    }
    let validators = Validators::new(validators);

    let header = MacroHeader {
        block_number: 32 * 60 - 1,
        validators: Some(validators),
        ..Default::default()
    };
    let block = Block::Macro(MacroBlock {
        header,
        body: None,
        justification: None,
    });

    let wire = block.serialize_to_vec();
    log::info!(
        wire_bytes = wire.len(),
        validators = n_validators,
        slots_per_validator = k,
        "crafted RequestBlock response"
    );
    assert!(
        wire.len() < 1_000_000,
        "wire payload must fit under the cap; got {}",
        wire.len()
    );

    match Block::deserialize_from_vec(&wire) {
        Ok(mut block) => {
            log::debug!("deserialized raw response; calling hash_cached() as request_block does");
            let _hash = block.hash_cached();
            panic!(
                "malicious validator set was NOT rejected at deserialize -- vulnerability present"
            );
        }
        Err(e) => {
            log::info!(error = ?e, "malicious response rejected at deserialize -- no allocation");
        }
    }
}
