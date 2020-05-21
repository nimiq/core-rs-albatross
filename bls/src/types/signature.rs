use std::fmt;

use algebra::mnt6_753::G1Projective;
use algebra_core::curves::ProjectiveCurve;

use crate::compression::BeSerialize;
use crate::{CompressedSignature, SigHash};
use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::primitives::{pedersen_generators, pedersen_hash};
use nano_sync::utils::bytes_to_bits;

#[derive(Clone, Copy)]
pub struct Signature {
    /// The projective form is the longer one, with three coordinates. The affine form is the shorter one, with only two coordinates. Calculation is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub signature: G1Projective,
}

impl Signature {
    /// Maps an hash to a elliptic curve point in the G1 group, it is known as "hash-to-curve". It
    /// is required to create signatures. We use the Pedersen hash to create the EC point.
    pub fn hash_to_g1(hash: SigHash) -> G1Projective {
        // Transform the hash into bits.
        let bits = bytes_to_bits(hash.as_bytes());

        // Get the generators for the Pedersen hash.
        let generators = pedersen_generators(256);

        // Calculate the Pedersen hash.
        let point = pedersen_hash(bits, generators, sum_generator_g1_mnt6());

        point
    }

    /// Transforms a signature into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine form),
    /// one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedSignature {
        let mut buffer = [0u8; 96];
        BeSerialize::serialize(&self.signature.into_affine(), &mut &mut buffer[..]).unwrap();
        CompressedSignature { signature: buffer }
    }
}

impl Eq for Signature {}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.signature.eq(&other.signature)
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.compress().to_hex())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Signature({})", &::hex::encode(self.compress().as_ref()))
    }
}
