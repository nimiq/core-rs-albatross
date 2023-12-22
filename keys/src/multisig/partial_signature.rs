use curve25519_dalek::Scalar;

use super::commitment::Commitment;
use crate::Signature;

/// A partial signature is a signature of one of the co-signers in a multisig.
/// Combining all partial signatures then yields the full signature (combining is done through summation).
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct PartialSignature(pub Scalar);
implement_simple_add_sum_traits!(PartialSignature, Scalar::ZERO);

impl PartialSignature {
    pub const SIZE: usize = 32;

    pub fn to_signature(&self, aggregated_commitment: &Commitment) -> Signature {
        let mut signature: [u8; Signature::SIZE] = [0u8; Signature::SIZE];
        signature[..Commitment::SIZE].copy_from_slice(&aggregated_commitment.to_bytes());
        signature[Commitment::SIZE..].copy_from_slice(self.as_bytes());
        Signature::from(&signature)
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; PartialSignature::SIZE] {
        self.0.as_bytes()
    }
}

impl From<[u8; PartialSignature::SIZE]> for PartialSignature {
    fn from(bytes: [u8; PartialSignature::SIZE]) -> Self {
        PartialSignature(Scalar::from_bytes_mod_order(bytes))
    }
}

impl<'a> From<&'a [u8; PartialSignature::SIZE]> for PartialSignature {
    fn from(bytes: &'a [u8; PartialSignature::SIZE]) -> Self {
        PartialSignature::from(*bytes)
    }
}
