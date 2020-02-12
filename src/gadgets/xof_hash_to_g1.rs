use algebra::curves::bls12_377::{
    g1::Bls12_377G1Parameters, Bls12_377Parameters, G1Affine, G1Projective as Bls12_377G1Projective,
};
use algebra::{
    curves::{models::SWModelParameters, ProjectiveCurve},
    fields::{bls12_377::Fq as Bls12_377Fp, sw6::Fr as SW6Fr},
    AffineCurve, BigInteger, BitIterator, One, PrimeField, Zero,
};
use r1cs_core::SynthesisError;
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::{
    alloc::AllocGadget,
    bits::ToBitsGadget,
    boolean::Boolean,
    eq::EqGadget,
    fields::FieldGadget,
    groups::{curves::short_weierstrass::bls12::G1Gadget, GroupGadget},
    Assignment,
};

use crate::gadgets::y_to_bit::YToBitGadget;

pub struct XofHashToG1Gadget {}

impl XofHashToG1Gadget {
    pub const LOOP_LIMIT: usize = 256;

    fn bits_to_fp(input_bits: &[Boolean]) -> Result<(Bls12_377Fp, bool), SynthesisError> {
        let mut bits = input_bits[..377]
            .iter()
            .map(|x| x.get_value().get())
            .collect::<Result<Vec<bool>, SynthesisError>>()?;
        bits.reverse();
        let big = <Bls12_377Fp as PrimeField>::BigInt::from_bits(&bits);
        let x = Bls12_377Fp::from_repr(big);
        let greatest = input_bits[377].get_value().get().unwrap();
        Ok((x, greatest))
    }

    /// Returns the nonce i, such that (x + i) is a valid x coordinate for G1.
    fn try_and_increment(x: Bls12_377Fp, y: bool) -> Option<Bls12_377Fp> {
        let mut i = Bls12_377Fp::zero();
        let mut valid = false;

        for _ in 0..Self::LOOP_LIMIT {
            let tmp = x + &i;
            if G1Affine::get_point_from_x(tmp, y).is_some() {
                valid = true;
                break;
            }
            i += &Bls12_377Fp::one();
        }
        if valid {
            Some(i)
        } else {
            None
        }
    }

    fn enforce_bit_slice_equality<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        slice1: &[Boolean],
        slice2: &[Boolean],
    ) -> Result<(), SynthesisError> {
        for (i, (a, b)) in slice1.iter().zip(slice2.iter()).enumerate() {
            a.enforce_equal(cs.ns(|| format!("enforce bit {}", i)), b)?;
        }
        Ok(())
    }

    pub fn hash_to_g1<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        xof_bits: &[Boolean],
    ) -> Result<G1Gadget<Bls12_377Parameters>, SynthesisError> {
        let xof_bits = &xof_bits[..377 + 1]; // Ignore unnecessary bits.
        let x_and_y = Self::bits_to_fp(xof_bits);

        // TODO: To reduce circuit size, we should do the addition of x and i on bit representations.
        // -- Beginning of x + i --
        // Allocate the Fp representation of `xof_bits`.
        let x_var = FpGadget::alloc(cs.ns(|| "Fp repr of hash"), || {
            let (x_val, _) = x_and_y
                .as_ref()
                .map_err(|_| SynthesisError::AssignmentMissing)?;
            Ok(x_val)
        })?;

        // Convert x_var to bits.
        let x_bits_var = x_var.to_bits(cs.ns(|| "serialized x_var"))?;
        // Enforce equality.
        Self::enforce_bit_slice_equality(cs.ns(|| "x bit equality"), xof_bits, &x_bits_var)?;

        // Allocate nonce of try + increment.
        let i_var = FpGadget::alloc(cs.ns(|| "try and increment nonce"), || {
            let (x_val, y_val) = x_and_y?;
            Self::try_and_increment(x_val, y_val).ok_or(SynthesisError::Unsatisfiable)
        })?;

        // Perform x + i and convert to bits.
        let point_x_var = x_var.add(cs.ns(|| "x + i"), &i_var)?;

        let point_bits = point_x_var.to_bits(cs.ns(|| "valid x point bits"))?;
        // -- End of x + i --

        // Convert i_var to bits and check that it is <= 256.
        let i_bits_var = i_var.to_bits(cs.ns(|| "serialized i_var"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Bls12_377Fp, _>(
            cs.ns(|| "i <= 256"),
            &i_bits_var,
            &[Self::LOOP_LIMIT as u64],
        )?;

        // Now, it is guaranteed that `point_bits` will result in a valid x coordinate.
        let expected_point_before_cofactor = G1Gadget::<Bls12_377Parameters>::alloc(
            cs.ns(|| "expected point before cofactor"),
            || {
                if point_bits.iter().any(|x| x.get_value().is_none()) {
                    Err(SynthesisError::AssignmentMissing)
                } else {
                    let (x, y) = Self::bits_to_fp(&point_bits)?;
                    let p = G1Affine::get_point_from_x(x, y).unwrap();
                    Ok(p.into_projective())
                }
            },
        )?;

        // We're not going to implement get_point_from_x inside the circuit.
        // Instead, we assume the given G1Gadget and calculate its bit representation in the CS.
        // Then, we compare it bit wise with the `point_bits`.
        let mut serialized_bits: Vec<Boolean> =
            expected_point_before_cofactor.x.to_bits(cs.ns(|| "bits"))?;
        serialized_bits.reverse();
        let greatest_bit = YToBitGadget::<Bls12_377Parameters>::y_to_bit_g1(
            cs.ns(|| "y to bit"),
            &expected_point_before_cofactor,
        )?;
        serialized_bits.push(greatest_bit);

        Self::enforce_bit_slice_equality(
            cs.ns(|| "point equality"),
            &serialized_bits,
            &point_bits,
        )?;

        let scaled_point = Self::scale_by_cofactor_g1(
            cs.ns(|| "scale by cofactor"),
            &expected_point_before_cofactor,
        )?;

        Ok(scaled_point)
    }

    fn scale_by_cofactor_g1<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        p: &G1Gadget<Bls12_377Parameters>,
    ) -> Result<G1Gadget<Bls12_377Parameters>, SynthesisError> {
        let generator = Bls12_377G1Projective::prime_subgroup_generator();
        let generator_var =
            G1Gadget::<Bls12_377Parameters>::alloc(cs.ns(|| "generator"), || Ok(generator))?;
        let mut x_bits = BitIterator::new(Bls12_377G1Parameters::COFACTOR)
            .map(|b| Boolean::constant(b))
            .collect::<Vec<Boolean>>();
        x_bits.reverse();
        let scaled = p
            .mul_bits(cs.ns(|| "scaled"), &generator_var, x_bits.iter())
            .unwrap()
            .sub(cs.ns(|| "scaled finalize"), &generator_var)
            .unwrap(); //x
        Ok(scaled)
    }
}
