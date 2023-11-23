use ark_ff::{One, PrimeField, ToConstraintField};
use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq, G1Affine};
use ark_r1cs_std::{
    fields::{fp::FpVar, FieldVar},
    prelude::{AllocVar, EqGadget},
    uint8::UInt8,
    R1CSVar, ToBitsGadget, ToConstraintFieldGadget,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_std::Zero;
use nimiq_hash::blake2s::Blake2xParameters;

use crate::{blake2s::evaluate_blake2s_with_parameters, gadgets::y_to_bit::YToBitGadget};

/// This gadget implements the functionality to hash a message into an elliptic curve point.
pub struct HashToCurve;

impl HashToCurve {
    pub fn hash_to_g1(
        cs: ConstraintSystemRef<MNT6Fq>,
        hash: &[UInt8<MNT6Fq>],
    ) -> Result<G1Var, SynthesisError> {
        // Extend the hash to 96 bytes using Blake2X.
        let mut hash_out = Vec::new();

        for i in 0..3 {
            // Initialize Blake2s parameters.
            let blake2s_parameters = Blake2xParameters::new(i, 0xffff);

            // Calculate hash.
            hash_out.extend(evaluate_blake2s_with_parameters(
                hash,
                &blake2s_parameters.parameters(),
            )?);
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

        // Separate the hash bits into coordinates.
        // The y-coordinate is at the most significant bit (interpreted as little endian). We convert it to a boolean.
        let bytes_len = hash_out.len();
        let y_bit = hash_out[bytes_len - 1].to_bits_le()?.pop().unwrap();

        // Because of the previous explanation, we need to remove the whole last two bytes.
        let max_size = ((MNT6Fq::MODULUS_BIT_SIZE - 1) / 8) as usize;
        hash_out.truncate(max_size);

        let mut x_coordinates = ToConstraintFieldGadget::to_constraint_field(&hash_out)?;
        assert_eq!(x_coordinates.len(), 1);
        let x_coordinate = x_coordinates.pop().unwrap();

        let mut g1_option = None;

        // Allocate the nonce bits and convert to a field element.
        let nonce = FpVar::<MNT6Fq>::new_witness(cs.clone(), || {
            let x_bytes = hash_out
                .iter()
                .map(|i| i.value())
                .collect::<Result<_, SynthesisError>>();
            let y = y_bit.value();

            // We need to always return a vector of the correct size for the setup to succeed (otherwise Vec::new_witness fails with an AssignmentMissing error).
            let (x_bytes, y) = match (x_bytes, y) {
                (Ok(x_bytes), Ok(y)) => (x_bytes, y),
                (..) => return Ok(MNT6Fq::zero()),
            };

            // Calculate the increment nonce and the resulting G1 hash point.
            let (nonce_byte, g1) = Self::try_and_increment(x_bytes, y);
            g1_option = Some(g1);
            Ok(MNT6Fq::from(nonce_byte))
        })?;

        // Allocate the G1 hash point.
        let g1_var = G1Var::new_witness(cs.clone(), || {
            g1_option.ok_or(SynthesisError::AssignmentMissing)
        })?;

        // Add the nonce to the x-coordinate.
        let x = x_coordinate + nonce;

        // Compare the coordinates of our G1 hash point to the calculated coordinates.
        let g1_var_affine = g1_var.to_affine()?;

        let y_coordinate = g1_var_affine.y_to_bit(cs)?;
        let coordinates = g1_var_affine.to_constraint_field()?;

        coordinates[0].enforce_equal(&x)?;
        y_coordinate.enforce_equal(&y_bit)?;
        coordinates[2].enforce_equal(&FpVar::zero())?;

        // We don't need to scale by the cofactor since MNT6-753 has a cofactor of one.

        // Return the hash point.
        Ok(g1_var)
    }

    /// Returns the nonce i, such that (x + i) is a valid x coordinate for G1.
    /// The nonce i is always a u8.
    fn try_and_increment(x_bytes: Vec<u8>, y: bool) -> (u8, G1Affine) {
        // Transform the x-coordinate into a field element.
        let x_coordinates = ToConstraintField::to_field_elements(&x_bytes).unwrap();
        assert_eq!(x_coordinates.len(), 1);
        let mut x_coordinate = x_coordinates[0];

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        for i in 0..=255 {
            let point = G1Affine::get_point_from_x_unchecked(x_coordinate, y);

            if let Some(g1) = point {
                // Note that we don't scale by the cofactor here. We do it later.
                return (i, g1);
            }

            x_coordinate += &MNT6Fq::one();
        }

        unreachable!()
    }
}
