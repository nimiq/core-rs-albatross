use ark_ec::{
    pairing::Pairing,
    short_weierstrass::{Affine, SWCurveConfig, SWFlags},
    AffineRepr,
};
use ark_ff::{Field, ToConstraintField};
use ark_groth16::VerifyingKey;

pub trait CompressedAffine<F: Field> {
    /// Returns the y-is-negative bit and the x coordinate.
    fn to_field_elements(&self) -> Option<(bool, Vec<F>)>;
}

pub trait CompressedComposite<F: Field> {
    /// Returns the y-is-negative bit and the x coordinate.
    fn to_field_elements(&self) -> Option<(Vec<u8>, Vec<F>)>;
    /// Returns a compressed serialized form.
    /// This consists of all y bits first, followed by all x coordinates.
    fn to_bytes(&self) -> Option<Vec<u8>> {
        let (mut bytes, elems) = self.to_field_elements()?;
        for elem in elems {
            elem.serialize_uncompressed(&mut bytes).ok()?;
        }
        Some(bytes)
    }
}

impl<M: SWCurveConfig, ConstraintF: Field> CompressedAffine<ConstraintF> for Affine<M>
where
    M::BaseField: ToConstraintField<ConstraintF>,
{
    #[inline]
    fn to_field_elements(&self) -> Option<(bool, Vec<ConstraintF>)> {
        if self.is_zero() {
            return None;
        }

        let x = ToConstraintField::<ConstraintF>::to_field_elements(&self.x)?;
        let y_bit = !SWFlags::from_y_coordinate(self.y).is_positive()?;
        Some((y_bit, x))
    }
}

impl<ConstraintF: Field, P: Pairing> CompressedComposite<ConstraintF> for VerifyingKey<P>
where
    P::G1Affine: CompressedAffine<ConstraintF>,
    P::G2Affine: CompressedAffine<ConstraintF>,
{
    fn to_field_elements(&self) -> Option<(Vec<u8>, Vec<ConstraintF>)> {
        let mut bits = vec![];
        let (y_bit, mut elems) = self.alpha_g1.to_field_elements()?;
        bits.push(y_bit);

        let (y_bit, mut elems_tmp) = self.beta_g2.to_field_elements()?;
        bits.push(y_bit);
        elems.append(&mut elems_tmp);

        let (y_bit, mut elems_tmp) = self.gamma_g2.to_field_elements()?;
        bits.push(y_bit);
        elems.append(&mut elems_tmp);

        let (y_bit, mut elems_tmp) = self.delta_g2.to_field_elements()?;
        bits.push(y_bit);
        elems.append(&mut elems_tmp);

        for elem in self.gamma_abc_g1.iter() {
            let (y_bit, mut elems_tmp) = elem.to_field_elements()?;
            bits.push(y_bit);
            elems.append(&mut elems_tmp);
        }

        // Transform vector of bits to vector of bytes.
        // We always start at the least significant bit,
        // because that's the order used in arkworks.
        let mut bytes = vec![];
        for byte_bits in bits.chunks(8) {
            let mut byte = 0;
            for (idx, &bit) in byte_bits.iter().enumerate() {
                if bit {
                    byte |= 1 << idx;
                }
            }
            bytes.push(byte);
        }

        Some((bytes, elems))
    }
}
