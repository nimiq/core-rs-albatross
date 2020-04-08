use algebra::Bls12_377;
use algebra_core::test_rng;
use groth16::{create_random_proof, generate_random_parameters};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::circuits::{DummyCircuit, OtherDummyCircuit, WrapperCircuit};

#[test]
fn everything_works() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_hash: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_hash.clone(), state_hash.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = WrapperCircuit::new(proof, parameters.vk, state_hash.clone(), state_hash);
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(test_cs.is_satisfied())
}

#[test]
fn different_state_hashes() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_hash: Vec<u8> = vec![0; 2];
    let other_state_hash: Vec<u8> = vec![1; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_hash.clone(), state_hash.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = WrapperCircuit::new(proof, parameters.vk, state_hash.clone(), other_state_hash);
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_verifying_key() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_hash: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_hash.clone(), state_hash.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit = OtherDummyCircuit::new(state_hash.clone(), state_hash.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = WrapperCircuit::new(proof, other_parameters.vk, state_hash.clone(), state_hash);
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_proof() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_hash: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_hash.clone(), state_hash.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit = OtherDummyCircuit::new(state_hash.clone(), state_hash.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = WrapperCircuit::new(other_proof, parameters.vk, state_hash.clone(), state_hash);
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}
