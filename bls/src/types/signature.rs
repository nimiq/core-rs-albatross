use std::fmt;

use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ec::ProjectiveCurve;
use ark_ff::{One, PrimeField};
use ark_mnt6_753::{Fq, Fq3, G2Affine, G2Projective};
use blake2_rfc::blake2s::Blake2s;

use nimiq_hash::HashOutput;

use crate::utils::big_int_from_bytes_be;
use crate::{CompressedSignature, SigHash};

#[derive(Clone, Copy)]
pub struct Signature {
    /// The projective form is the longer one, with three coordinates. The
    /// affine form is the shorter one, with only two coordinates. Calculation
    /// is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support
    /// arithmetic with it.
    pub signature: G2Projective,
    /// Cache for the compressed form of the signature. This is done in order
    /// to optimize the serialization since it needs to be compressed for this
    /// purpose.
    pub compressed: CompressedSignature,
}

impl Signature {
    /// Maps an hash to a elliptic curve point, it is known as
    /// "hash-to-curve". It is required to create signatures. We use the
    /// try-and-increment method to create the EC point.
    pub fn hash_to_point(hash: SigHash) -> G2Projective {
        // This extends the seed using the Blake2X algorithm.
        // See https://blake2.net/blake2x.pdf for more details.
        // We need 288 bytes of output for the generator that we are going to create.
        let mut bytes = vec![];

        for i in 0..9 {
            let blake2x = Blake2sWithParameterBlock {
                digest_length: 32,
                key_length: 0,
                fan_out: 0,
                depth: 0,
                leaf_length: 32,
                node_offset: i as u32,
                xof_digest_length: 65535,
                node_depth: 0,
                inner_length: 32,
                salt: [0; 8],
                personalization: [0; 8],
            };

            let mut state = Blake2s::with_parameter_block(&blake2x.parameters());

            state.update(hash.as_bytes());

            let mut result = state.finalize().as_bytes().to_vec();

            bytes.append(&mut result);
        }

        // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve
        // point. At this time, it is not guaranteed to be a valid point. A quirk of this code is that
        // we need to set the most significant 16 bits to zero. The reason for this is that the field for
        // the MNT6-753 curve is not exactly 753 bits, it is a bit smaller. This means that if we try
        // to create a field element from 753 random bits, we may get an invalid value back (in this
        // case it is just all zeros). There are two options to deal with this:
        // 1) To create finite field elements, using 753 random bits, in a loop until a valid one
        //    is created.
        // 2) Use only 752 random bits to create a finite field element. This will guaranteedly
        //    produce a valid element on the first try, but will reduce the entropy of the EC
        //    point generation by one bit.
        // We chose the second one because we believe the entropy reduction is not significant enough.
        // Since we have 768 bits per generator but only need 752 bits, we set the first 16 bits (768-752=16)
        // to zero.
        // The y-coordinate is at the first bit. We convert it to a boolean.
        let y_coordinate = (bytes[0] >> 7) & 1 == 1;

        // In order to easily read the BigInt from the bytes, we use the first 16 bits as padding.
        // However, because of the previous explanation, we need to nullify the whole first two bytes.
        bytes[0] = 0;
        bytes[1] = 0;

        bytes[96] = 0;
        bytes[97] = 0;

        bytes[192] = 0;
        bytes[193] = 0;

        let c0 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[..96])).unwrap();
        let c1 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[96..192])).unwrap();
        let c2 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[192..])).unwrap();
        let mut x_coordinate = Fq3::new(c0, c1, c2);

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G2Affine::get_point_from_x(x_coordinate, y_coordinate);

            if let Some(point) = point {
                // Scale the point by the cofactor of MNT6-753 before returning.
                return point.scale_by_cofactor();
            }

            x_coordinate += &Fq3::one();
        }
    }

    /// Transforms a signature into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine
    /// form), one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedSignature {
        self.compressed
    }

    /// Multiplies a Signature by a u16. It's useful when you need to
    /// validator's signature by its number of slots.
    #[must_use]
    pub fn multiply(&self, x: u16) -> Self {
        let signature = self.signature.mul(&[x as u64]);
        Signature {
            signature,
            compressed: CompressedSignature::from(signature),
        }
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

impl From<G2Projective> for Signature {
    fn from(signature: G2Projective) -> Self {
        let compressed = CompressedSignature::from(signature);

        Signature {
            signature,
            compressed,
        }
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    // TODO: Replace this with a generic serialization using `ToHex` and `FromHex`.

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::{CompressedSignature, Signature};

    impl Serialize for Signature {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Serialize::serialize(&self.compress(), serializer)
        }
    }

    impl<'de> Deserialize<'de> for Signature {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let compressed: CompressedSignature = Deserialize::deserialize(deserializer)?;
            compressed.uncompress().map_err(Error::custom)
        }
    }
}
