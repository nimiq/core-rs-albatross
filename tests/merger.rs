use algebra::Bls12_377;
use algebra_core::test_rng;
use groth16::{create_random_proof, generate_random_parameters};
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

use nano_sync::circuits::{DummyCircuit, MergerCircuit, OtherDummyCircuit};

#[test]
fn everything_works() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(other_dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        proof,
        other_proof,
        parameters.vk,
        other_parameters.vk,
        state_commitment.clone(),
        state_commitment.clone(),
        state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(test_cs.is_satisfied())
}

#[test]
fn different_state_commitmentes() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];
    let other_state_commitment: Vec<u8> = vec![1; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(other_dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        proof,
        other_proof,
        parameters.vk,
        other_parameters.vk,
        state_commitment.clone(),
        state_commitment,
        other_state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_intermediate_state_commitment() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];
    let other_state_commitment: Vec<u8> = vec![1; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(other_dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        proof,
        other_proof,
        parameters.vk,
        other_parameters.vk,
        other_state_commitment,
        state_commitment.clone(),
        state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_first_verifying_key() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(other_dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        proof,
        other_proof,
        other_parameters.clone().vk,
        other_parameters.vk,
        state_commitment.clone(),
        state_commitment.clone(),
        state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_second_verifying_key() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(other_dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        proof,
        other_proof,
        parameters.clone().vk,
        parameters.vk,
        state_commitment.clone(),
        state_commitment.clone(),
        state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_first_proof() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the other dummy circuit.
    let other_proof = create_random_proof(other_dummy_circuit, &other_parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        other_proof.clone(),
        other_proof,
        parameters.vk,
        other_parameters.vk,
        state_commitment.clone(),
        state_commitment.clone(),
        state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_second_proof() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let state_commitment: Vec<u8> = vec![0; 2];

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<Bls12_377, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Create other dummy circuit.
    let other_dummy_circuit =
        OtherDummyCircuit::new(state_commitment.clone(), state_commitment.clone());

    // Generate parameters for the other dummy circuit.
    let other_parameters =
        generate_random_parameters::<Bls12_377, _, _>(other_dummy_circuit.clone(), rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = MergerCircuit::new(
        proof.clone(),
        proof,
        parameters.vk,
        other_parameters.vk,
        state_commitment.clone(),
        state_commitment.clone(),
        state_commitment,
    );
    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}
