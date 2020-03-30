use algebra::Bls12_377;
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;

use algebra::bls12_377::Fq;
use algebra_core::test_rng;
use groth16::{create_random_proof, generate_random_parameters, Parameters, Proof};
use nano_sync::circuits::{DummyCircuit, WrapperCircuit};

// Create the dummy circuit parameters and proof.
fn setup(state_hash: Vec<u8>) -> (Parameters<Bls12_377>, Proof<Bls12_377>) {
    let rng = &mut test_rng();

    let circuit = DummyCircuit::new(state_hash.clone(), state_hash);

    let params = generate_random_parameters::<Bls12_377, _, _>(circuit.clone(), rng).unwrap();

    let proof = create_random_proof(circuit, &params, rng).unwrap();

    (params, proof)
}

//#[test]
fn everything_works() {
    // Create public input.
    let state_hash: Vec<u8> = vec![0; 2];

    // Create parameters and proof for a dummy circuit.
    let (parameters, proof) = setup(state_hash.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let circuit = WrapperCircuit::new(proof, parameters.vk, state_hash.clone(), state_hash);

    circuit.generate_constraints(&mut test_cs).unwrap();

    assert!(test_cs.is_satisfied())
}
