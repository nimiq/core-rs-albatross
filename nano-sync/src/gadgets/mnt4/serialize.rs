use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, G2Var};
use ark_r1cs_std::prelude::{Boolean, ToBitsGadget};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::YToBitGadget;
use crate::utils::pad_point_bits;

/// A gadget that takes as input a G1 or G2 point and serializes it into a vector of Booleans.
pub struct SerializeGadget;

impl SerializeGadget {
    pub fn serialize_g1(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &G1Var,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Convert the point to affine coordinates.
        let aff_point = point.to_affine()?;

        // Get bits from the x coordinate.
        let x_bits = aff_point.x.to_bits_le()?;

        // Get one bit from the y coordinate.
        let y_bit = YToBitGadget::y_to_bit_g1(cs, &aff_point)?;

        // Get the infinity flag.
        let infinity_bit = aff_point.infinity;

        // Pad points and get *Big-Endian* representation.
        let bits = pad_point_bits::<MNT4Fr>(x_bits, y_bit, infinity_bit);

        Ok(bits)
    }

    pub fn serialize_g2(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &G2Var,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Convert the point to affine coordinates.
        let aff_point = point.to_affine()?;

        // Get bits from the x coordinate.
        let x_bits = aff_point.x.to_bits_le()?;

        // Get one bit from the y coordinate.
        let y_bit = YToBitGadget::y_to_bit_g2(cs, &aff_point)?;

        // Get the infinity flag.
        let infinity_bit = aff_point.infinity;

        // Pad points and get *Big-Endian* representation.
        let bits = pad_point_bits::<MNT4Fr>(x_bits, y_bit, infinity_bit);

        Ok(bits)
    }
}
