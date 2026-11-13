use nimiq_bls::CompressedSignature;

/// Returns compressed signature bytes that aren't a curve point: x = 1 with the flags cleared.
/// They pass the flag and range checks and only fail at the square root.
pub fn signature_not_on_curve() -> CompressedSignature {
    let mut signature = CompressedSignature::default();
    signature.signature[0] = 1;
    signature
}
