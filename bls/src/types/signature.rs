use crate::compression::BeSerialize;

use super::*;

#[derive(Clone, Copy)]
pub struct Signature {
    /// The projective form is the longer one, with three coordinates. The affine form is the shorter one, with only two coordinates. Calculation is faster with the projective form.
    /// We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub signature: G1Projective,
}

impl Signature {
    /// Maps an hash to a elliptic curve point in the G1 group. It is required to create signatures.
    /// It is also known as "hash-to-curve".
    pub fn hash_to_g1(h: SigHash) -> G1Projective {
        // This extends the input hash from 32 bytes to 48 bytes using the Blake2X algorithm.
        // See https://blake2.net/blake2x.pdf for more details.
        let mut bytes = vec![];
        let digest_length = vec![32, 16];
        for i in 0..2 {
            let blake2x = Blake2sWithParameterBlock {
                digest_length: digest_length[i],
                key_length: 0,
                fan_out: 0,
                depth: 0,
                leaf_length: 32,
                node_offset: i as u32,
                xof_digest_length: 48,
                node_depth: 0,
                inner_length: 32,
                salt: [0; 8],
                personalization: [0; 8],
            };
            let mut state = Blake2s::with_parameter_block(&blake2x.parameters());
            state.update(h.as_bytes());
            let mut result = state.finalize().as_bytes().to_vec();
            bytes.append(&mut result);
        }

        // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve point. At this time, it is not guaranteed to be a valid point.
        // A quirk of this code is that we need to set the most significant bit to zero. The reason for this is that the field for the BLS12-377 curve is not exactly 377 bits, it is a bit smaller.
        // This means that if we try to create a field element from 377 random bits, we may get an invalid value back (in this case it is just all zeros). There are two options to deal with this:
        // 1) To create finite field elements, using 377 random bits, in a loop until a valid one is created.
        // 2) Use only 376 random bits to create a finite field element. This will guaranteedly produce a valid element on the first try, but will reduce the entropy of the EC point generation by one bit.
        // We chose the second one because we believe the entropy reduction is not significant enough.
        // The y-coordinate is at first bit.
        let y_coordinate = (bytes[0] >> 7) & 1 == 1;
        // In order to easily read the BigInt from the bytes, we use the first 7 bits as padding.
        // However, because of the previous explanation, we also need to set the 8th bit to 0.
        // Thus, we can nullify the whole first byte.
        bytes[0] = 0;
        let mut x_coordinate = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[..]));

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);
            if point.is_some() {
                let point = G1Affine::from(point.unwrap());
                let g1 = point.scale_by_cofactor();
                return g1;
            }
            x_coordinate += &Fq::one();
        }
    }

    /// Transforms a signature into a serialized compressed form.
    /// This form consists of the x-coordinate of the point (in the affine form),
    /// one bit indicating the sign of the y-coordinate
    /// and one bit indicating if it is the "point-at-infinity".
    pub fn compress(&self) -> CompressedSignature {
        let mut buffer = [0u8; 48];
        BeSerialize::serialize(
            &self.signature.into_affine(),
            &mut &mut buffer[..]
        ).unwrap();
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
