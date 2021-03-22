use ark_ec::mnt6::MNT6Parameters;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{Fq3Var, FqVar};
use ark_mnt6_753::Parameters;
use ark_r1cs_std::groups::curves::short_weierstrass::AffineVar;
use ark_r1cs_std::prelude::Boolean;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::y_to_bit::YToBitGadget as Y2BG;

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub type YToBitGadget = Y2BG<MNT4Fr>;

impl YToBitGadget {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g1(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &AffineVar<<Parameters as MNT6Parameters>::G1Parameters, FqVar>,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        let y_bit = Self::is_greater_half(cs, &point.y)?;

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g2(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &AffineVar<<Parameters as MNT6Parameters>::G2Parameters, Fq3Var>,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        // Calculate the required inputs to the formula.
        let y_c2_bit = Self::is_greater_half(cs.clone(), &point.y.c2)?;

        let y_c1_bit = Self::is_greater_half(cs.clone(), &point.y.c1)?;

        let y_c0_bit = Self::is_greater_half(cs.clone(), &point.y.c0)?;

        let y_c2_eq_bit = Self::is_equal_zero(cs.clone(), &point.y.c2)?;

        let y_c1_eq_bit = Self::is_equal_zero(cs, &point.y.c1)?;

        // Calculate the following formula:
        // (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
        let cond0 = y_c2_bit;

        let cond1 = Boolean::and(&y_c2_eq_bit, &y_c1_bit)?;

        let cond2 = Boolean::kary_and(vec![y_c2_eq_bit, y_c1_eq_bit, y_c0_bit].as_ref())?;

        let y_bit = Boolean::kary_or(vec![cond0, cond1, cond2].as_ref())?;

        Ok(y_bit)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::Fr as MNT4Fr;
    use ark_mnt6_753::constraints::{G1Var, G2Var};
    use ark_mnt6_753::{G1Projective, G2Projective};
    use ark_r1cs_std::prelude::AllocVar;
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};

    use nimiq_bls::utils::bytes_to_bits;
    use nimiq_nano_primitives::{serialize_g1_mnt6, serialize_g2_mnt6};

    use super::*;

    #[test]
    fn y_to_bit_g1_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        for _ in 0..10 {
            // Create random point.
            let g1_point = G1Projective::rand(rng);

            // Allocate the random inputs in the circuit and convert it to affine form.
            let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point))
                .unwrap()
                .to_affine()
                .unwrap();

            // Serialize using the primitive version and get the first bit (which is the y flag).
            let bytes = serialize_g1_mnt6(g1_point);
            let bits = bytes_to_bits(&bytes);
            let primitive_y_bit = bits[0];

            // Serialize using the gadget version and get the boolean value.
            let gadget_y_bit = YToBitGadget::y_to_bit_g1(cs.clone(), &g1_point_var)
                .unwrap()
                .value()
                .unwrap();

            assert_eq!(primitive_y_bit, gadget_y_bit);
        }
    }

    #[test]
    fn y_to_bit_g2_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        for _ in 0..10 {
            // Create random point.
            let g2_point = G2Projective::rand(rng);

            // Allocate the random inputs in the circuit and convert it to affine form.
            let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point))
                .unwrap()
                .to_affine()
                .unwrap();

            // Serialize using the primitive version and get the first bit (which is the y flag).
            let bytes = serialize_g2_mnt6(g2_point);
            let bits = bytes_to_bits(&bytes);
            let primitive_y_bit = bits[0];

            // Serialize using the gadget version and get the boolean value.
            let gadget_y_bit = YToBitGadget::y_to_bit_g2(cs.clone(), &g2_point_var)
                .unwrap()
                .value()
                .unwrap();

            assert_eq!(primitive_y_bit, gadget_y_bit);
        }
    }
}
