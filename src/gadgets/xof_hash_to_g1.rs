use algebra::curves::bls12_377::{
    g1::Bls12_377G1Parameters, Bls12_377Parameters, G1Affine, G1Projective as Bls12_377G1Projective,
};
use algebra::{
    curves::{models::SWModelParameters, ProjectiveCurve},
    fields::{bls12_377::Fq as Bls12_377Fp, sw6::Fr as SW6Fr},
    AffineCurve, BigInteger, BitIterator, One, PrimeField, Zero,
};
use r1cs_core::SynthesisError;
use r1cs_std::{
    alloc::AllocGadget,
    bits::ToBitsGadget,
    boolean::Boolean,
    eq::EqGadget,
    groups::{curves::short_weierstrass::bls12::G1Gadget, GroupGadget},
    Assignment,
};

use crate::gadgets::bytes_to_bits;
use crate::gadgets::constant::AllocConstantGadget;
use crate::gadgets::y_to_bit::YToBitGadget;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

pub struct XofHashToG1Gadget {}

impl XofHashToG1Gadget {
    fn bits_to_fp(
        input_bits: &[Boolean],
        y_bit: &Boolean,
    ) -> Result<(Bls12_377Fp, bool), SynthesisError> {
        assert_eq!(input_bits.len(), 377, "Expected 377 bit input");
        // If we take 377 bits, chances are that the resulting value is larger than the modulus.
        // This will cause the `from_repr` call to return 0.
        // To prevent this, we instead give up one bit of entropy and set the most significant bit
        // to false.
        let mut bits = input_bits
            .iter()
            .map(|x| x.get_value().get())
            .collect::<Result<Vec<bool>, SynthesisError>>()?;
        bits[0] = false; // Big-Endian, so 0 is the highest bit.
        let big = <Bls12_377Fp as PrimeField>::BigInt::from_bits(&bits);
        let x = Bls12_377Fp::from_repr(big);
        let greatest = y_bit.get_value().get()?;
        Ok((x, greatest))
    }

    /// Returns the nonce i (as a vector of big endian bits), such that (x + i) is a valid x coordinate for G1.
    fn try_and_increment(x: Bls12_377Fp, y: bool) -> Option<Vec<bool>> {
        let mut i = Bls12_377Fp::zero();
        let mut i_byte: u8 = 0;
        let mut valid = false;

        for _ in 0..256 {
            let tmp = x + &i;
            if G1Affine::get_point_from_x(tmp, y).is_some() {
                valid = true;
                break;
            }
            i += &Bls12_377Fp::one();
            i_byte += 1;
        }
        if valid {
            Some(bytes_to_bits(&[i_byte]))
        } else {
            None
        }
    }

    fn enforce_bit_slice_equality<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        slice1: &[Boolean],
        slice2: &[Boolean],
    ) -> Result<(), SynthesisError> {
        assert_eq!(
            slice1.len(),
            slice2.len(),
            "bit slices should be of same size"
        );
        for (i, (a, b)) in slice1.iter().zip(slice2.iter()).enumerate() {
            a.enforce_equal(cs.ns(|| format!("enforce bit {}", i)), b)?;
        }
        Ok(())
    }

    pub fn hash_to_g1<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        xof_bits: &[Boolean],
    ) -> Result<G1Gadget<Bls12_377Parameters>, SynthesisError> {
        assert_eq!(xof_bits.len(), 384); // 377 rounded to the next byte.

        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc nonce");

        // We skip the first 7 bits, as this will make it easier in the non ZK-proof version.
        // The reason for this is that we can set the first 7 bits to 0 there and get a byte-aligned
        // number to read in.
        let mut x_bits = vec![Boolean::constant(false)];
        x_bits.extend_from_slice(&xof_bits[8..384]);
        let y_bit = &xof_bits[0];
        let nonce_bits =
            Self::bits_to_fp(&x_bits, y_bit).and_then(|(x, y)| Self::try_and_increment(x, y).get());

        // Allocate the bits of the nonce of try + increment.
        let mut nonce_var = vec![];
        for _ in 0..369 {
            nonce_var.push(Boolean::constant(false));
        }
        for k in 369..377 {
            nonce_var.push(Boolean::alloc(
                cs.ns(|| format!("allocating bit {} for try and increment nonce", k)),
                || {
                    let nonce_val = nonce_bits
                        .as_ref()
                        .map_err(|_| SynthesisError::AssignmentMissing)?;
                    Ok(nonce_val[k - 369])
                },
            )?);
        }

        // Perform x + i and append y_bit.
        next_cost_analysis!(cs, cost, || "Binary adder x + nonce");
        let mut point_bits = Boolean::binary_adder(
            cs.ns(|| "binary addition of x coordinate and nonce"),
            &x_bits,
            &nonce_var,
        )?;
        point_bits.push(y_bit.clone()); // We add the y_bit for later slice comparison.

        // Now, it is guaranteed that `point_bits` will result in a valid x coordinate.
        next_cost_analysis!(cs, cost, || "Allocate expected point");
        let expected_point_before_cofactor = G1Gadget::<Bls12_377Parameters>::alloc(
            cs.ns(|| "expected point before cofactor"),
            || {
                if point_bits.iter().any(|x| x.get_value().is_none()) {
                    Err(SynthesisError::AssignmentMissing)
                } else {
                    // Remember to remove the y_bit.
                    let (x, y) = Self::bits_to_fp(&point_bits[..377], y_bit)?;
                    let p = G1Affine::get_point_from_x(x, y).unwrap();
                    Ok(p.into_projective())
                }
            },
        )?;

        // We're not going to implement get_point_from_x inside the circuit.
        // Instead, we assume the given G1Gadget and calculate its bit representation in the CS.
        // Then, we compare it bit wise with the `point_bits`.
        next_cost_analysis!(cs, cost, || "Serialize expected point");
        let mut serialized_bits: Vec<Boolean> =
            expected_point_before_cofactor.x.to_bits(cs.ns(|| "bits"))?;
        let greatest_bit = YToBitGadget::<Bls12_377Parameters>::y_to_bit_g1(
            cs.ns(|| "y to bit"),
            &expected_point_before_cofactor,
        )?;
        serialized_bits.push(greatest_bit);

        next_cost_analysis!(cs, cost, || "Enforce equality with bit result");
        Self::enforce_bit_slice_equality(
            cs.ns(|| "point equality"),
            &serialized_bits,
            &point_bits,
        )?;

        next_cost_analysis!(cs, cost, || "Scale by cofactor");
        let scaled_point = Self::scale_by_cofactor_g1(
            cs.ns(|| "scale by cofactor"),
            &expected_point_before_cofactor,
        )?;
        end_cost_analysis!(cs, cost);
        Ok(scaled_point)
    }

    fn scale_by_cofactor_g1<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        p: &G1Gadget<Bls12_377Parameters>,
    ) -> Result<G1Gadget<Bls12_377Parameters>, SynthesisError> {
        // TODO: Do not allocate a new generator for every call.
        let generator = Bls12_377G1Projective::prime_subgroup_generator();
        let generator_var =
            G1Gadget::<Bls12_377Parameters>::alloc_const(cs.ns(|| "generator"), &generator)?;
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
