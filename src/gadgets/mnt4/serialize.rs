use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::FqParameters;
use r1cs_core::SynthesisError;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use r1cs_std::ToBitsGadget;

use crate::gadgets::mnt4::YToBitGadget;
use crate::utils::pad_point_bits;

/// A gadget that takes as input a G1 or G2 point and serializes it into a vector of Booleans.
pub struct SerializeGadget;

impl SerializeGadget {
    pub fn serialize_g1<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        point: &G1Gadget,
    ) -> Result<Vec<Boolean>, SynthesisError> {
        // Get bits from the x coordinate.
        let x_bits = point.x.to_bits(cs.ns(|| "x to bits"))?;

        // Get one bit from the y coordinate.
        let y_bit = YToBitGadget::y_to_bit_g1(cs.ns(|| "y to bit"), point)?;

        // Pad points and get *Big-Endian* representation.
        let bits = pad_point_bits::<FqParameters>(x_bits, y_bit);

        Ok(bits)
    }

    pub fn serialize_g2<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        point: &G2Gadget,
    ) -> Result<Vec<Boolean>, SynthesisError> {
        // Get bits from the x coordinate.
        let x_bits = point.x.to_bits(cs.ns(|| "x to bits"))?;

        // Get one bit from the y coordinate.
        let y_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| "y to bit"), point)?;

        // Pad points and get *Big-Endian* representation.
        let bits = pad_point_bits::<FqParameters>(x_bits, y_bit);

        Ok(bits)
    }
}
