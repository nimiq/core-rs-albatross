use ark_crypto_primitives::prf::blake2s::constraints::evaluate_blake2s_with_parameters;
use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ff::{One, PrimeField};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{Fq3Var, G2Var};
use ark_mnt6_753::{Fq, Fq3, G2Affine};

use ark_r1cs_std::fields::FieldVar;
use ark_r1cs_std::prelude::{AllocVar, Boolean, CondSelectGadget, EqGadget};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use nimiq_bls::utils::{big_int_from_bytes_be, byte_from_be_bits, byte_to_le_bits};

use crate::gadgets::mnt4::YToBitGadget;
use crate::utils::reverse_inner_byte_order;

/// This gadget implements the functionality to hash a message into an elliptic curve point.
pub struct HashToCurve;

impl HashToCurve {
    pub fn hash_to_g2(
        cs: ConstraintSystemRef<MNT4Fr>,
        hash: &[Boolean<MNT4Fr>],
    ) -> Result<G2Var, SynthesisError> {
        // Extend the hash to 288 bytes using Blake2X.
        let mut hash_out = Vec::new();

        for i in 0..9 {
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

        let mut c0_bits = vec![Boolean::constant(false); 16];
        c0_bits.extend_from_slice(&hash_bits[2 * 8..96 * 8]);

        let mut c1_bits = vec![Boolean::constant(false); 16];
        c1_bits.extend_from_slice(&hash_bits[(96 + 2) * 8..192 * 8]);

        let mut c2_bits = vec![Boolean::constant(false); 16];
        c2_bits.extend_from_slice(&hash_bits[(192 + 2) * 8..]);

        // Calculate the increment nonce and the resulting G1 hash point.
        let (nonce_bits, hash_point) = Self::try_and_increment(
            c0_bits.iter().map(|i| i.value().unwrap()).collect(),
            c1_bits.iter().map(|i| i.value().unwrap()).collect(),
            c2_bits.iter().map(|i| i.value().unwrap()).collect(),
            y_bit.value()?,
        );

        // Allocate the nonce bits.
        let nonce_bits_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(nonce_bits))?;

        // Allocate the hash point.
        let hash_var = G2Var::new_witness(cs.clone(), || Ok(hash_point))?;

        // Convert the x-coordinate bits into a field element.
        c0_bits.reverse();
        let c0 = Boolean::le_bits_to_fp_var(&c0_bits)?;

        c1_bits.reverse();
        let c1 = Boolean::le_bits_to_fp_var(&c1_bits)?;

        c2_bits.reverse();
        let c2 = Boolean::le_bits_to_fp_var(&c2_bits)?;

        let x = Fq3Var::new(c0, c1, c2);

        // Convert the nonce bits into a field element. Using double-and-add.
        let mut nonce = Fq3Var::zero();
        let mut power = Fq3Var::one();

        for bit in nonce_bits_var {
            let new_nonce = &nonce + &power;

            nonce = Fq3Var::conditionally_select(&bit, &new_nonce, &nonce)?;

            power = power.double()?;
        }

        // Add the nonce to the x-coordinate.
        let x = x + nonce;

        // Compare the coordinates of our hash point to the calculated coordinates.
        let hash_var_affine = hash_var.to_affine()?;

        let y_coordinate = YToBitGadget::y_to_bit_g2(cs, &hash_var_affine)?;
        let x_coordinate = hash_var_affine.x;
        let inf_coordinate = hash_var_affine.infinity;

        x_coordinate.enforce_equal(&x)?;
        y_coordinate.enforce_equal(y_bit)?;
        inf_coordinate.enforce_equal(&Boolean::constant(false))?;

        // TODO! The hash point still needs to be scaled by the cofactor of the curve. Otherwise the
        //  signature won't verify.
        // We now scale by the cofactor.
        //let scaled_hash_var = hash_var_affine.fixed_scalar_mul_le()?;

        // Return the hash point.
        Ok(hash_var)
    }

    /// Returns the nonce i (as a vector of big endian bits), such that (x + i) is a valid x coordinate for G2.
    fn try_and_increment(
        c0_bits: Vec<bool>,
        c1_bits: Vec<bool>,
        c2_bits: Vec<bool>,
        y: bool,
    ) -> (Vec<bool>, G2Affine) {
        // Prepare the bits to transform into field element.
        let mut c0_bytes = vec![];
        let mut c1_bytes = vec![];
        let mut c2_bytes = vec![];

        for i in 0..96 {
            c0_bytes.push(byte_from_be_bits(&c0_bits[i * 8..(i + 1) * 8]));
            c1_bytes.push(byte_from_be_bits(&c1_bits[i * 8..(i + 1) * 8]));
            c2_bytes.push(byte_from_be_bits(&c2_bits[i * 8..(i + 1) * 8]));
        }

        // Transform the x-coordinate into a field element.
        let c0 = Fq::from_repr(big_int_from_bytes_be(&mut &c0_bytes[..])).unwrap();
        let c1 = Fq::from_repr(big_int_from_bytes_be(&mut &c1_bytes[..])).unwrap();
        let c2 = Fq::from_repr(big_int_from_bytes_be(&mut &c2_bytes[..])).unwrap();
        let mut x = Fq3::new(c0, c1, c2);

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        for i in 0..=255 {
            let point = G2Affine::get_point_from_x(x, y);

            if let Some(g2) = point {
                let i_bits = byte_to_le_bits(i);
                // Note that we don't scale by the cofactor here. We do it later.
                return (i_bits, g2);
            }

            x += &Fq3::one();
        }

        unreachable!()
    }
}
