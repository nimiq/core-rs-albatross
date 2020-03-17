#![allow(dead_code)]

use std::env;
use std::error::Error;
use std::str::FromStr;

use algebra::bls12_377::{Fq, FqParameters};
use algebra::bls12_377::{G2Projective, Parameters as Bls12_377Parameters};
use algebra::ProjectiveCurve;
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget;
use nimiq_bls::{KeyPair, SecureGenerate};
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::alloc::AllocGadget;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::bits::uint32::UInt32;
use r1cs_std::fields::FieldGadget;
use r1cs_std::fields::FqGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::G2Gadget;
use r1cs_std::groups::GroupGadget;
use r1cs_std::test_constraint_system::TestConstraintSystem;
use r1cs_std::ToBitsGadget;

use nano_sync::gadgets::alloc_constant::AllocConstantGadget;
use nano_sync::gadgets::y_to_bit::YToBitGadget;
use nano_sync::gadgets::{pad_point_bits, reverse_inner_byte_order};

fn sum_keys_and_hash<CS: ConstraintSystem<Fq>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc(cs.ns(|| "public keys"), || {
        Ok(&public_keys[..])
    })?;

    let mut sum: G2Gadget<Bls12_377Parameters> = G2Gadget::alloc_const(
        cs.ns(|| "generator"),
        &G2Projective::prime_subgroup_generator(),
    )?;

    // Add all public keys.
    for (i, key) in keys_var.iter().enumerate() {
        sum = sum.add(cs.ns(|| format!("sum {}", i)), key)?;
    }

    let serialized_bits: Vec<Boolean> = sum.x.to_bits(cs.ns(|| "bits"))?;
    let greatest_bit =
        YToBitGadget::<Bls12_377Parameters>::y_to_bit_g2(cs.ns(|| "y to bit"), &sum)?;

    // Pad points and get *Big-Endian* representation.
    let serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);

    // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
    let serialized_bits = reverse_inner_byte_order(&serialized_bits);

    // Hash serialized bits.
    blake2s_gadget(cs.ns(|| "h0 from serialized bits"), &serialized_bits)?;
    Ok(())
}

fn only_blake2s_circuit<CS: ConstraintSystem<Fq>>(
    mut cs: CS,
    public_keys: &[G2Projective],
) -> Result<(), SynthesisError> {
    let keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc(cs.ns(|| "public keys"), || {
        Ok(&public_keys[..])
    })?;

    let mut bits = vec![];
    // Just append all bit representations.
    for (i, key) in keys_var.iter().enumerate() {
        let serialized_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| format!("bits {}", i)))?;
        let greatest_bit = YToBitGadget::<Bls12_377Parameters>::y_to_bit_g2(
            cs.ns(|| format!("y to bit {}", i)),
            key,
        )?;

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

fn blake2s_circuit<CS: ConstraintSystem<Fq>>(
    mut cs: CS,
    bits: &[bool],
    rounds: usize,
) -> Result<usize, SynthesisError> {
    let bits_var = Vec::<Boolean>::alloc(cs.ns(|| "bits"), || Ok(&bits[..]))?;
    let setup_constraints = cs.num_constraints();

    // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
    let bits_var = reverse_inner_byte_order(&bits_var);

    // Hash serialized bits.
    for i in 0..rounds {
        blake2s_gadget(cs.ns(|| format!("hash {}", i)), &bits_var)?;
    }

    Ok(cs.num_constraints() - setup_constraints)
}

fn main() -> Result<(), Box<dyn Error>> {
    let keys: Vec<_> = env::args().collect();
    let mut num_keys = keys.get(1).map(|s| usize::from_str(&s).ok()).flatten();
    if num_keys.is_none() {
        println!("You can set a custom number of keys to be generated via command line arguments.");
        num_keys = Some(10);
    }
    let num_keys = num_keys.unwrap();

    let mut public_keys = vec![];

    println!("Generate {} keys", num_keys);
    for _ in 0..num_keys {
        let key_pair = KeyPair::generate_default_csprng();
        public_keys.push(key_pair.public_key.public_key);
    }

    println!("Only Blake2s:");
    let mut cs = TestConstraintSystem::new();
    only_blake2s_circuit(cs.ns(|| "circuit"), &public_keys)?;
    println!("Number of constraints: {}", cs.num_constraints());

    println!("Sum, then Blake2s:");
    let mut cs = TestConstraintSystem::new();
    sum_keys_and_hash(cs.ns(|| "circuit"), &public_keys)?;
    println!("Number of constraints: {}", cs.num_constraints());

    println!();
    println!("Two Blake2s vs. one Blake2s with double input size");
    for i in 1..4 {
        let bits = vec![true; 32 * 8 * i];
        let mut cs = TestConstraintSystem::new();
        let constraints = blake2s_circuit(cs.ns(|| "circuit"), &bits, 2)?;
        println!(
            "Number of constraints 2x Blake2s, {} byte: {}",
            32 * i,
            constraints
        );

        let bits = vec![true; 32 * 8 * 2 * i];
        let mut cs = TestConstraintSystem::new();
        let constraints = blake2s_circuit(cs.ns(|| "circuit"), &bits, 1)?;
        println!(
            "Number of constraints 1x Blake2s, {} byte: {}",
            32 * 2 * i,
            constraints
        );
    }

    println!();
    println!("Hashing 133 = 32 + 96 + 4 + 1 bytes");
    println!("h(133) vs. h(32 + 4 + 1) xor h(96)");
    let bits = vec![true; 133 * 8];
    let mut cs = TestConstraintSystem::new();
    let constraints = blake2s_circuit(cs.ns(|| "circuit"), &bits, 1)?;
    println!("Number of constraints h(133): {}", constraints);

    let mut cs = TestConstraintSystem::new();
    let bits = vec![true; (32 + 4 + 1) * 8];
    let constraints1 = blake2s_circuit(cs.ns(|| "circuit1"), &bits, 1)?;
    let bits = vec![true; 96 * 8];
    let constraints2 = blake2s_circuit(cs.ns(|| "circuit2"), &bits, 1)?;
    let bit1 = Boolean::alloc(cs.ns(|| "bit1"), || Ok(true))?;
    let bit2 = Boolean::alloc(cs.ns(|| "bit2"), || Ok(true))?;
    let constraints_before = cs.num_constraints();
    Boolean::xor(cs.ns(|| "xor"), &bit1, &bit2)?;
    let constraints3 = (cs.num_constraints() - constraints_before) * 32 * 8;
    println!(
        "Number of constraints h(32 + 4 + 1) xor h(96): {}",
        constraints1 + constraints2 + constraints3
    );
    println!("Number of constraints h(32 + 4 + 1): {}", constraints1);
    println!("Number of constraints h(96): {}", constraints2);
    println!("Number of constraints xor: {}", constraints3);

    println!();
    println!("Addition on Fq vs. UInt32");
    let mut cs = TestConstraintSystem::new();
    let zero = FqGadget::zero(cs.ns(|| "zero"))?;
    let epoch = FqGadget::alloc_const(cs.ns(|| "epoch"), &Fq::from(128u64))?;
    let constraints = cs.num_constraints();
    zero.add(cs.ns(|| "addition"), &epoch)?;
    println!(
        "Number of constraints Fq add: {}",
        cs.num_constraints() - constraints
    );
    let mut cs = TestConstraintSystem::new();
    let zero = FqGadget::zero(cs.ns(|| "zero"))?;
    let constraints = cs.num_constraints();
    zero.add_constant(cs.ns(|| "addition"), &Fq::from(128u64))?;
    println!(
        "Number of constraints Fq add_constant: {}",
        cs.num_constraints() - constraints
    );
    let mut cs = TestConstraintSystem::<Fq>::new();
    let zero = UInt32::constant(0);
    let epoch = UInt32::constant(128);
    UInt32::addmany(cs.ns(|| "addition"), &[zero, epoch])?;
    println!("Number of constraints UInt32: {}", cs.num_constraints());

    Ok(())
}
