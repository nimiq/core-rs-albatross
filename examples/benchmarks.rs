#![allow(dead_code)]

// For benchmarking
use std::error::Error;

use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{FqParameters, G2Projective};
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget;
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, Boolean, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use r1cs_std::ToBitsGadget;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::mnt4::{MNT4YToBitGadget, PedersenCommitmentGadget};
use nano_sync::gadgets::{pad_point_bits, reverse_inner_byte_order};
use nano_sync::primitives::mnt4::pedersen_generators;
use nimiq_bls::{KeyPair, SecureGenerate};

fn main() -> Result<(), Box<dyn Error>> {
    // Setup keys.
    let key_pair1 = KeyPair::generate_default_csprng();
    let keys = vec![key_pair1.public_key.public_key; 1];

    // All Blake2s.
    println!("Only Blake2s:");
    let mut cs = TestConstraintSystem::new();
    only_blake2s_circuit(cs.ns(|| "circuit 1"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

    // All Pedersen.
    println!("Only Pedersen:");
    let mut cs = TestConstraintSystem::new();
    only_pedersen_circuit(cs.ns(|| "circuit 2"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

    // Only serialization.
    println!("Only serialization:");
    let mut cs = TestConstraintSystem::new();
    only_serialize_pks(cs.ns(|| "circuit 3"), &keys.clone())?;
    println!("Number of constraints: {}", cs.num_constraints());

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
        let greatest_bit = MNT4YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bit {}", i)), key)?;

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

fn only_pedersen_circuit<CS: ConstraintSystem<MNT4Fr>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget>::alloc(cs.ns(|| "public keys"), || Ok(&public_keys[..]))?;

    let mut bits = vec![];
    // Just append all bit representations.
    for (i, key) in keys_var.iter().enumerate() {
        let serialized_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| format!("bits {}", i)))?;
        let greatest_bit = MNT4YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bit {}", i)), key)?;

        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);
        bits.append(&mut serialized_bits);
    }

    let sum_generator = sum_generator_g1_mnt6();

    let sum_generator_var = G1Gadget::alloc(cs.ns(|| "sum generator"), || Ok(sum_generator))?;

    let generators_needed = (bits.len() + 752 - 1) / 752;
    let generators = pedersen_generators(generators_needed);

    let mut pedersen_generators_var: Vec<G1Gadget> = Vec::new();
    for i in 0..generators.len() {
        pedersen_generators_var.push(G1Gadget::alloc(
            cs.ns(|| format!("pedersen_generators: generator {}", i)),
            || Ok(generators[i]),
        )?);
    }

    // Calculate the Pedersen commitment.
    let pedersen_commitment = PedersenCommitmentGadget::evaluate(
        cs.ns(|| "pedersen commitment"),
        &pedersen_generators_var,
        &bits,
        &sum_generator_var,
    )?;

    // Serialize the Pedersen commitment.
    let x_bits = pedersen_commitment
        .x
        .to_bits(cs.ns(|| "x to bits: pedersen commitment"))?;
    let greatest_bit = MNT4YToBitGadget::y_to_bit_g1(
        cs.ns(|| "y to bit: pedersen commitment"),
        &pedersen_commitment,
    )?;
    let serialized_bits = pad_point_bits::<FqParameters>(x_bits, greatest_bit);

    // Convert to bytes.
    let mut bytes = Vec::new();
    for i in 0..serialized_bits.len() / 8 {
        bytes.push(UInt8::from_bits_le(&serialized_bits[i..i + 8]));
    }

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
        let greatest_bit = MNT4YToBitGadget::y_to_bit_g2(cs.ns(|| format!("y to bit {}", i)), key)?;

        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);
        bits.append(&mut serialized_bits);
    }

    Ok(())
}
