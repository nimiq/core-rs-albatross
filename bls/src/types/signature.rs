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
    pub(crate) fn hash_to_g1(h: SigHash) -> G1Projective {
        let rng = &mut ChaChaRng::from_seed(h.into());
        loop {
            let x_coordinate = Fq::rand(rng);
            let y_coordinate = bool::rand(rng);
            let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);
            if point.is_some() {
                let point = G1Affine::from(point.unwrap());
                let g1 = point.scale_by_cofactor();
                return g1;
            }
        }
    }

    /// Transforms a signature into a serialized compressed form. This form consists of the x-coordinate of the point (in the affine form), one bit indicating the sign of the y-coordinate, one bit indicating if it is the "point-at-infinity" and one bit indicating that this is the compressed form.
    pub fn compress(&self) -> CompressedSignature {
        let mut buffer = [0u8; 48];
        self.signature
            .into_affine()
            .serialize(&[], &mut buffer)
            .unwrap();
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
