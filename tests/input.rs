use algebra::{MNT4_753, MNT6_753};
use algebra_core::{test_rng, PairingEngine, ToConstraintField};
use crypto_primitives::nizk::groth16::constraints::{
    Groth16VerifierGadget, ProofGadget, VerifyingKeyGadget,
};
use crypto_primitives::nizk::Groth16;
use crypto_primitives::NIZKVerifierGadget;
use groth16::{
    create_random_proof, generate_random_parameters, prepare_verifying_key, verify_proof, Proof,
    VerifyingKey,
};
use nano_sync::gadgets::input::RecursiveInputGadget;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::alloc::AllocGadget;
use r1cs_std::bits::uint8::UInt8;
use r1cs_std::eq::EqGadget;
use r1cs_std::mnt4_753::PairingGadget;
use r1cs_std::test_constraint_system::TestConstraintSystem;

#[derive(Clone)]
struct DummyCircuit {
    input_bytes: Vec<u8>,
}

impl ConstraintSynthesizer<<MNT4_753 as PairingEngine>::Fr> for DummyCircuit {
    fn generate_constraints<CS: ConstraintSystem<<MNT4_753 as PairingEngine>::Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate bytes in private.
        let private_bytes = UInt8::alloc_vec(cs.ns(|| "Private input"), &self.input_bytes)?;
        // Allocate bytes from public input.
        let public_bytes = UInt8::alloc_input_vec(cs.ns(|| "Public input"), &self.input_bytes)?;
        // Check equality.
        for (i, (a, b)) in private_bytes.iter().zip(public_bytes.iter()).enumerate() {
            a.enforce_equal(cs.ns(|| format!("Check eq {}", i)), b)?;
        }
        Ok(())
    }
}

// Renaming some types for convenience. We can change the circuit and elliptic curve of the input
// proof to the wrapper circuit just by editing these types.
type TheProofSystem = Groth16<MNT4_753, DummyCircuit, <MNT4_753 as PairingEngine>::Fr>;
type TheProofGadget = ProofGadget<MNT4_753, <MNT6_753 as PairingEngine>::Fr, PairingGadget>;
type TheVkGadget = VerifyingKeyGadget<MNT4_753, <MNT6_753 as PairingEngine>::Fr, PairingGadget>;
type TheVerifierGadget =
    Groth16VerifierGadget<MNT4_753, <MNT6_753 as PairingEngine>::Fr, PairingGadget>;

#[derive(Clone)]
struct VerifierCircuit {
    input_bytes: Vec<u8>,
    proof: Proof<MNT4_753>,
    vk: VerifyingKey<MNT4_753>,
}

impl ConstraintSynthesizer<<MNT6_753 as PairingEngine>::Fr> for VerifierCircuit {
    fn generate_constraints<CS: ConstraintSystem<<MNT6_753 as PairingEngine>::Fr>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate bytes from public input.
        let public_bytes = UInt8::alloc_input_vec(cs.ns(|| "Public input"), &self.input_bytes)?;

        // Allocate verifying key (in a productive environment, this should be allocated as a constant).
        let vk = TheVkGadget::alloc(cs.ns(|| "VK"), || Ok(&self.vk))?;
        // Allocate the proof (in a productive environment, this should be allocated as a constant).
        let proof = TheProofGadget::alloc(cs.ns(|| "Proof"), || Ok(&self.proof))?;

        // Here comes the conversion.
        let input_bytes = RecursiveInputGadget::to_field_elements::<<MNT4_753 as PairingEngine>::Fr>(
            &public_bytes,
        )?;

        <TheVerifierGadget as NIZKVerifierGadget<
            TheProofSystem,
            <MNT6_753 as PairingEngine>::Fr,
        >>::check_verify(
            cs.ns(|| "Verify groth16 proof"),
            &vk,
            input_bytes.iter(),
            &proof,
        )?;

        Ok(())
    }
}

#[test]
fn recursive_input_is_read_correctly() {
    // Initialize RNG.
    let rng = &mut test_rng();

    // Create public input.
    let mut input_bytes: Vec<u8> = Vec::new();
    for i in 0..200 {
        input_bytes.push(i);
    }

    // Create dummy circuit.
    let dummy_circuit = DummyCircuit {
        input_bytes: input_bytes.clone(),
    };

    // Generate parameters for the dummy circuit.
    let parameters =
        generate_random_parameters::<MNT4_753, _, _>(dummy_circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(dummy_circuit, &parameters, rng).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();
    let circuit = VerifierCircuit {
        input_bytes: input_bytes.clone(),
        proof,
        vk: parameters.vk,
    };
    circuit.clone().generate_constraints(&mut test_cs).unwrap();

    assert!(test_cs.is_satisfied());

    // Also test using real Groth16.
    // Generate parameters for the verifying circuit.
    let parameters = generate_random_parameters::<MNT6_753, _, _>(circuit.clone(), rng).unwrap();

    // Generate a proof for the dummy circuit.
    let proof = create_random_proof(circuit, &parameters, rng).unwrap();
    let pvk = prepare_verifying_key(&parameters.vk);

    // Pack inputs.
    let fes = input_bytes.to_field_elements().unwrap();
    assert!(verify_proof(&pvk, &proof, &fes).unwrap())
}
