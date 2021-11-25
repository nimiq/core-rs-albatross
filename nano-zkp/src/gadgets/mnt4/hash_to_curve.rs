use ark_crypto_primitives::prf::blake2s::constraints::evaluate_blake2s_with_parameters;
use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ff::{One, PrimeField};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_mnt6_753::G1Affine;
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::fields::FieldVar;
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget};
use ark_r1cs_std::{R1CSVar, ToBitsGadget, ToConstraintFieldGadget};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use nimiq_bls::utils::{big_int_from_bytes_be, byte_from_be_bits, byte_to_le_bits};

use crate::gadgets::mnt4::YToBitGadget;
use crate::utils::reverse_inner_byte_order;

/// This gadget implements the functionality to hash a message into an elliptic curve point.
pub struct HashToCurve;

impl HashToCurve {
    pub fn hash_to_g1(
        cs: ConstraintSystemRef<MNT4Fr>,
        hash: &Vec<Boolean<MNT4Fr>>,
    ) -> Result<G1Var, SynthesisError> {
        // Extend the hash to 96 bytes using Blake2X.
        let mut hash_out = Vec::new();

        for i in 0..3 {
            // Initialize Blake2s parameters.
            let blake2s_parameters = Blake2sWithParameterBlock {
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

            // Calculate hash.
            hash_out.extend(evaluate_blake2s_with_parameters(
                hash,
                &blake2s_parameters.parameters(),
            )?);
        }

        // Convert to bits.
        let mut hash_bits = Vec::new();

        for int in &hash_out {
            hash_bits.extend(int.to_bits_le());
        }

        // Reverse inner byte order again to get in the correct order. At this point the hash should
        // match the off-circuit one.
        let hash_bits = reverse_inner_byte_order(&hash_bits);

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
        let y_bit = &hash_bits[0];

        let mut x_bits = vec![Boolean::constant(false); 16];
        x_bits.extend_from_slice(&hash_bits[16..768]);

        // Calculate the increment nonce and the resulting G1 hash point.
        let (nonce_bits, g1) = Self::try_and_increment(
            x_bits.iter().map(|i| i.value().unwrap()).collect(),
            y_bit.value()?,
        );

        // Allocate the nonce bits and convert to a field element.
        let nonce_bits_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(nonce_bits))?;
        let nonce = Boolean::le_bits_to_fp_var(&nonce_bits_var)?;

        // Allocate the G1 hash point.
        let g1_var = G1Var::new_witness(cs.clone(), || Ok(g1))?;

        // Convert the x-coordinate bits into a field element.
        x_bits.reverse();
        let x = Boolean::le_bits_to_fp_var(&x_bits)?;

        // Add the nonce to the x-coordinate.
        let x = x + nonce;

        // Compare the coordinates of our G1 hash point to the calculated coordinates.
        let g1_var_affine = g1_var.to_affine()?;

        let y_coordinate = YToBitGadget::y_to_bit_g1(cs, &g1_var_affine)?;
        let coordinates = g1_var_affine.to_constraint_field()?;

        coordinates[0].enforce_equal(&x)?;
        y_coordinate.enforce_equal(y_bit)?;
        coordinates[2].enforce_equal(&FpVar::zero())?;

        // We don't need to scale by the cofactor since MNT6-753 has a cofactor of one.

        // Return the hash point.
        Ok(g1_var)
    }

    /// Returns the nonce i (as a vector of big endian bits), such that (x + i) is a valid x coordinate for G1.
    fn try_and_increment(x_bits: Vec<bool>, y: bool) -> (Vec<bool>, G1Affine) {
        // Prepare the bits to transform into field element.
        let mut bytes = vec![];

        for i in 0..96 {
            bytes.push(byte_from_be_bits(&x_bits[i * 8..(i + 1) * 8]))
        }

        // Transform the x-coordinate into a field element.
        let mut x = MNT4Fr::from_repr(big_int_from_bytes_be(&mut &bytes[..])).unwrap();

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        let mut i: u8 = 0;

        for _ in 0..256 {
            let point = G1Affine::get_point_from_x(x, y);

            if point.is_some() {
                let i_bits = byte_to_le_bits(i);
                // Note that we don't scale by the cofactor here. We do it later.
                let g1 = point.unwrap();
                return (i_bits, g1);
            }

            x += &MNT4Fr::one();
            i += 1;
        }

        unreachable!()
    }
}
