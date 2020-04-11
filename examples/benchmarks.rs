#![allow(dead_code)]

// For benchmarking
use std::error::Error;

use algebra::mnt4_753::Fr as MNT4Fr;
use algebra_core::{One, PrimeField};
use blake2_rfc::blake2s::Blake2s;
use nimiq_bls::{big_int_from_bytes_be, KeyPair, SecureGenerate};
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::test_constraint_system::TestConstraintSystem;

use algebra::mnt6_753::{Fq, FqParameters, G1Affine, G1Projective, G2Projective};
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nano_sync::constants::{sum_generator_g1_mnt6, sum_generator_g2_mnt6};
use nano_sync::rand_gen::generate_random_seed;
use nano_sync::*;
use r1cs_std::mnt6_753::{G1Gadget, G1PreparedGadget, G2Gadget, G2PreparedGadget, PairingGadget};
use r1cs_std::pairing::PairingGadget as PG;
use r1cs_std::prelude::{AllocGadget, Boolean, CondSelectGadget, GroupGadget};
use r1cs_std::ToBitsGadget;
use std::borrow::Borrow;

fn main() -> Result<(), Box<dyn Error>> {
    // Setup keys.
    let key_pair1 = KeyPair::generate_default_csprng();
    let keys = vec![key_pair1.public_key.public_key; 1];

    // Only serialization.
    println!("Only serialization:");
    let mut cs = TestConstraintSystem::new();
    only_serialize_pks(cs.ns(|| "circuit 0"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

    // Full serialization, without y-to-bit.
    println!("Full serialization:");
    let mut cs = TestConstraintSystem::new();
    fully_serialize_pks(cs.ns(|| "circuit -1"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

    // All Blake2s.
    println!("Only Blake2s:");
    let mut cs = TestConstraintSystem::new();
    only_blake2s_circuit(cs.ns(|| "circuit 1"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

    // Only allocate generators.
    println!("Only alloc generators:");
    let mut cs = TestConstraintSystem::new();
    only_allocate_generators(cs.ns(|| "circuit 69"))?;
    println!("Number of constraints: {}", cs.num_constraints());

    // All Pedersen.
    println!("Only Pedersen:");
    let mut cs = TestConstraintSystem::new();
    only_pedersen_circuit(cs.ns(|| "circuit 2"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

    // Pairing.
    println!("Pairing:");
    let mut cs = TestConstraintSystem::new();
    pairing_circuit(cs.ns(|| "circuit 3"))?;
    println!("Number of constraints: {}", cs.num_constraints());

    Ok(())
}

fn pairing_circuit<CS: ConstraintSystem<MNT4Fr>>(mut cs: CS) -> Result<(), SynthesisError> {
    let sum_generator_g1 = sum_generator_g1_mnt6();

    let sum_generator_g1_var =
        G1Gadget::alloc(cs.ns(|| "sum generator 1"), || Ok(sum_generator_g1))?;

    let prepared_g1: G1PreparedGadget =
        PairingGadget::prepare_g1(cs.ns(|| "sig_p"), &sum_generator_g1_var)?;

    let sum_generator_g2 = sum_generator_g2_mnt6();

    let sum_generator_g2_var =
        G2Gadget::alloc(cs.ns(|| "sum generator 2"), || Ok(sum_generator_g2))?;

    let prepared_g2: G2PreparedGadget =
        PairingGadget::prepare_g2(cs.ns(|| "sig_addp"), &sum_generator_g2_var)?;

    let _par = PairingGadget::pairing(cs.ns(|| "pairing"), prepared_g1, prepared_g2)?;

    Ok(())
}

fn only_blake2s_circuit<CS: ConstraintSystem<MNT4Fr>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget>::alloc(cs.ns(|| "public keys"), || Ok(&public_keys[..]))?;

    let mut bits = vec![];
    // Just append all bit representations.
    for (i, key) in keys_var.iter().enumerate() {
        let serialized_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| format!("bits {}", i)))?;
        let greatest_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bit {}", i)), key)?;

        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);
        bits.append(&mut serialized_bits);
    }

    // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
    let serialized_bits = reverse_inner_byte_order(&bits);

    // Hash serialized bits.
    blake2s_gadget(cs.ns(|| "h0 from serialized bits"), &serialized_bits)?;

    Ok(())
}

fn only_serialize_pks<CS: ConstraintSystem<MNT4Fr>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget>::alloc(cs.ns(|| "public keys"), || Ok(&public_keys[..]))?;

    let mut bits = vec![];
    // Just append all bit representations.
    for (i, key) in keys_var.iter().enumerate() {
        let serialized_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| format!("bits {}", i)))?;
        let greatest_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bit {}", i)), key)?;

        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);
        bits.append(&mut serialized_bits);
    }

    Ok(())
}

fn fully_serialize_pks<CS: ConstraintSystem<MNT4Fr>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget>::alloc(cs.ns(|| "public keys"), || Ok(&public_keys[..]))?;

    let mut bits = vec![];
    // Just append all bit representations.
    for (i, key) in keys_var.iter().enumerate() {
        let mut serialized_bits_x: Vec<Boolean> =
            key.x.to_bits(cs.ns(|| format!("x bits {}", i)))?;
        let mut serialized_bits_y: Vec<Boolean> =
            key.y.to_bits(cs.ns(|| format!("y bits {}", i)))?;
        bits.append(&mut serialized_bits_x);
        bits.append(&mut serialized_bits_y);
    }

    Ok(())
}

fn only_pedersen_circuit<CS: ConstraintSystem<MNT4Fr>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget>::alloc(cs.ns(|| "public keys"), || Ok(&public_keys[..]))?;

    let mut bits = vec![];
    // Just append all bit representations.
    for (i, key) in keys_var.iter().enumerate() {
        let serialized_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| format!("bits {}", i)))?;
        let greatest_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bit {}", i)), key)?;

        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);
        bits.append(&mut serialized_bits);
    }

    let sum_generator = sum_generator_g1_mnt6();

    let sum_generator_var = G1Gadget::alloc(cs.ns(|| "sum generator"), || Ok(sum_generator))?;

    let mut result = sum_generator_var.clone();

    let pedersen_generators = setup_pedersen();
    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..pedersen_generators.len() {
        pedersen_generators_var.push(G1Gadget::alloc(
            cs.ns(|| format!("pedersen_generators: generator {}", i)),
            || Ok(pedersen_generators[i]),
        )?);
    }

    for i in 0..bits.len() {
        // Add the next generator to the current sum.
        let new_sum = result.add(
            cs.ns(|| format!("add bit {}", i)),
            &pedersen_generators_var[i],
        )?;
        // If the bit is zero, keep the current sum. If it is one, take the new sum.
        result = G1Gadget::conditionally_select(
            &mut cs.ns(|| format!("Conditional Select {}", i)),
            bits[i].borrow(),
            &new_sum,
            &result,
        )?;
    }

    Ok(())
}

fn only_allocate_generators<CS: ConstraintSystem<MNT4Fr>>(
    mut cs: CS,
) -> Result<(), SynthesisError> {
    let sum_generator = sum_generator_g1_mnt6();

    let _sum_generator_var = G1Gadget::alloc(cs.ns(|| "sum generator"), || Ok(sum_generator))?;

    let pedersen_generators = setup_pedersen();
    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..pedersen_generators.len() {
        pedersen_generators_var.push(G1Gadget::alloc(
            cs.ns(|| format!("pedersen_generators: generator {}", i)),
            || Ok(pedersen_generators[i]),
        )?);
    }

    Ok(())
}

/// The size of the input to the Pedersen hash, in bytes. It must be know ahead of time in order to
/// create the vector of generators. We need one generator per bit of input (8 per byte).
pub const INPUT_SIZE: usize = 285;

/// This is the function for generating the Pedersen hash generators for our specific instance. We
/// need one generator per each bit of input.
pub fn setup_pedersen() -> Vec<G1Projective> {
    // This gets a verifiably random seed.
    let seed = generate_random_seed();

    // This extends the seed using the Blake2X algorithm.
    // See https://blake2.net/blake2x.pdf for more details.
    // We need 96 bytes of output for each generator that we are going to create.
    // The number of rounds is calculated so that we get 48 bytes per generator needed. We use the
    // following formula for the ceiling division: |x/y| = (x+y-1)/y
    let mut bytes = vec![];
    let number_rounds = (INPUT_SIZE * 8 * 96 + 32 - 1) / 32;
    for i in 0..number_rounds {
        let blake2x = Blake2sWithParameterBlock {
            digest_length: 32,
            key_length: 0,
            fan_out: 0,
            depth: 0,
            leaf_length: 32,
            node_offset: i as u32,
            xof_digest_length: 65535,
            node_depth: 0,
            inner_length: 32,
            salt: [1; 8],
            personalization: [1; 8], // We set this to be different from the one used to generate the sum generators in constants.rs
        };
        let mut state = Blake2s::with_parameter_block(&blake2x.parameters());
        state.update(&seed);
        let mut result = state.finalize().as_bytes().to_vec();
        bytes.append(&mut result);
    }

    // Initialize the vector that will contain the generators.
    let mut generators = Vec::new();

    // Generating the generators.
    for i in 0..INPUT_SIZE * 8 {
        // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve point. At this time, it is not guaranteed to be a valid point.
        // A quirk of this code is that we need to set the most significant bit to zero. The reason for this is that the field for the BLS12-377 curve is not exactly 377 bits, it is a bit smaller.
        // This means that if we try to create a field element from 377 random bits, we may get an invalid value back (in this case it is just all zeros). There are two options to deal with this:
        // 1) To create finite field elements, using 377 random bits, in a loop until a valid one is created.
        // 2) Use only 376 random bits to create a finite field element. This will guaranteedly produce a valid element on the first try, but will reduce the entropy of the EC point generation by one bit.
        // We chose the second one because we believe the entropy reduction is not significant enough.
        // The y-coordinate is at first bit.
        let y_coordinate = (bytes[96 * i] >> 7) & 1 == 1;

        // In order to easily read the BigInt from the bytes, we use the first 7 bits as padding.
        // However, because of the previous explanation, we also need to set the 8th bit to 0.
        // Thus, we can nullify the whole first byte.
        bytes[96 * i] = 0;
        bytes[96 * i + 1] = 0;
        let mut x_coordinate =
            Fq::from_repr(big_int_from_bytes_be(&mut &bytes[96 * i..96 * (i + 1)]));

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);
            if point.is_some() {
                let g1 = point.unwrap().scale_by_cofactor();
                generators.push(g1);
                break;
            }
            x_coordinate += &Fq::one();
        }
    }

    generators
}
