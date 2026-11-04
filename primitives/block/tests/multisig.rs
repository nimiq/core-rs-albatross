//! A `MultiSignature`'s signer set is never hash-committed and its aggregate signature does not
//! depend on the encoding, so trailing zero limbs carry no information. The decoder drops them:
//! otherwise any peer could inflate a stored justification or a handel contribution. A macro
//! header's punished set is hash-committed and keeps its padding, as it must.

use nimiq_block::{MacroHeader, MultiSignature, SkipBlockProof, TendermintProof};
use nimiq_collections::BitSet;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::bitset::with_trailing_zero_limbs;

const PADDING_LIMBS: usize = 64;

/// A padded multi-signature and its canonical twin: same slots, same signature.
fn padded_and_canonical() -> (MultiSignature, MultiSignature) {
    let canonical = MultiSignature::new(Default::default(), BitSet::from_iter([0, 3, 511]));
    let padded = MultiSignature::new(
        Default::default(),
        with_trailing_zero_limbs(&canonical.signers, PADDING_LIMBS),
    );
    assert!(!padded.signers.is_canonical());
    assert_eq!(padded, canonical);
    (padded, canonical)
}

#[test]
fn decoding_drops_signer_set_padding() {
    let (padded, canonical) = padded_and_canonical();
    let bytes = padded.serialize_to_vec();
    assert_eq!(
        bytes.len(),
        canonical.serialize_to_vec().len() + PADDING_LIMBS
    );

    let decoded = MultiSignature::deserialize_from_vec(&bytes).unwrap();
    assert!(decoded.signers.is_canonical());
    assert_eq!(decoded, canonical);
    assert_eq!(decoded.serialize_to_vec(), canonical.serialize_to_vec());
    // The padding's allocation is released as well, not just truncated.
    assert!(decoded.signers.capacity() <= canonical.signers.capacity());
}

/// Both justification types carry a `MultiSignature`, so both come out canonical.
#[test]
fn decoding_drops_justification_padding() {
    let (padded, canonical) = padded_and_canonical();

    let proof = TendermintProof {
        round: 7,
        sig: padded.clone(),
    };
    let decoded = TendermintProof::deserialize_from_vec(&proof.serialize_to_vec()).unwrap();
    assert!(decoded.sig.signers.is_canonical());
    assert_eq!(
        decoded,
        TendermintProof {
            round: 7,
            sig: canonical.clone(),
        }
    );

    let proof = SkipBlockProof { sig: padded };
    let decoded = SkipBlockProof::deserialize_from_vec(&proof.serialize_to_vec()).unwrap();
    assert!(decoded.sig.signers.is_canonical());
    assert_eq!(decoded, SkipBlockProof { sig: canonical });
}

/// The punished set is hash-committed, so its padding must survive decoding byte for byte.
#[test]
fn decoding_keeps_punished_set_padding() {
    let header = MacroHeader {
        next_batch_initial_punished_set: with_trailing_zero_limbs(
            &BitSet::from_iter([5]),
            PADDING_LIMBS,
        ),
        ..Default::default()
    };
    let bytes = header.serialize_to_vec();
    let decoded = MacroHeader::deserialize_from_vec(&bytes).unwrap();
    assert!(!decoded.next_batch_initial_punished_set.is_canonical());
    assert_eq!(decoded.serialize_to_vec(), bytes);
}
