use algebra::sw6::Fr as SW6Fr;
use algebra::test_rng;
use algebra_core::ProjectiveCurve;
use crypto_primitives::crh::pedersen::constraints::PedersenCRHGadget;
use crypto_primitives::crh::pedersen::{PedersenCRH, PedersenParameters, PedersenWindow};
use crypto_primitives::{FixedLengthCRH, FixedLengthCRHGadget};
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem};
use r1cs_std::prelude::{AllocGadget, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;

use algebra::bls12_377::G1Projective;
use nano_sync::gadgets::{CRHGadget, CRHGadgetParameters};
use nano_sync::primitives::{setup_crh, CRHWindow, CRH};
use nano_sync::*;
use r1cs_std::bls12_377::G1Gadget;
use rand::Rng;

// When running tests you are advised to set VALIDATOR_SLOTS in constants.rs to a more manageable number, for example 4.

#[test]
fn crh_works() {
    let input: Vec<u8> = vec![
        0, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 106, 72, 248, 27, 130, 191, 148, 63, 76, 70, 8, 226, 110, 233, 33,
        218, 140, 85, 240, 233, 175, 186, 125, 167, 20, 109, 224, 253, 159, 65, 126, 198, 14, 119,
        100, 241, 66, 15, 61, 177, 242, 37, 74, 159, 237, 72, 51, 0, 197, 229, 178, 114, 41, 110,
        142, 168, 13, 217, 63, 133, 238, 215, 0, 100, 255, 128, 113, 228, 155, 194, 137, 51, 196,
        233, 154, 146, 156, 13, 32, 162, 1, 10, 176, 205, 181, 169, 236, 113, 7, 209, 107, 247, 4,
        21, 218, 0, 106, 72, 248, 27, 130, 191, 148, 63, 76, 70, 8, 226, 110, 233, 33, 218, 140,
        85, 240, 233, 175, 186, 125, 167, 20, 109, 224, 253, 159, 65, 126, 198, 14, 119, 100, 241,
        66, 15, 61, 177, 242, 37, 74, 159, 237, 72, 51, 0, 197, 229, 178, 114, 41, 110, 142, 168,
        13, 217, 63, 133, 238, 215, 0, 100, 255, 128, 113, 228, 155, 194, 137, 51, 196, 233, 154,
        146, 156, 13, 32, 162, 1, 10, 176, 205, 181, 169, 236, 113, 7, 209, 107, 247, 4, 21, 218,
        0, 106, 72, 248, 27, 130, 191, 148, 63, 76, 70, 8, 226, 110, 233, 33, 218, 140, 85, 240,
        233, 175, 186, 125, 167, 20, 109, 224, 253, 159, 65, 126, 198, 14, 119, 100, 241, 66, 15,
        61, 177, 242, 37, 74, 159, 237, 72, 51, 0, 197, 229, 178, 114, 41, 110, 142, 168, 13, 217,
        63, 133, 238, 215, 0, 100, 255, 128, 113, 228, 155, 194, 137, 51, 196, 233, 154, 146, 156,
        13, 32, 162, 1, 10, 176, 205, 181, 169, 236, 113, 7, 209, 107, 247, 4, 21, 218, 0, 106, 72,
        248, 27, 130, 191, 148, 63, 76, 70, 8, 226, 110, 233, 33, 218, 140, 85, 240, 233, 175, 186,
        125, 167, 20, 109, 224, 253, 159, 65, 126, 198, 14, 119, 100, 241, 66, 15, 61, 177, 242,
        37, 74, 159, 237, 72, 51, 0, 197, 229, 178, 114, 41, 110, 142, 168, 13, 217, 63, 133, 238,
        215, 0, 100, 255, 128, 113, 228, 155, 194, 137, 51, 196, 233, 154, 146, 156, 13, 32, 162,
        1, 10, 176, 205, 181, 169, 236, 113, 7, 209, 107, 247, 4, 21, 218,
    ];
    let input: Vec<u8> = vec![
        0, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 106, 72, 248, 27, 130, 191, 148, 63, 76, 70, 8, 226, 110, 233, 33,
        218, 140, 85, 240, 233, 175, 186, 125, 167, 20, 109, 224, 253, 159, 65, 126, 198, 14, 119,
        100, 241, 66, 15, 61, 177, 242, 37, 74, 159, 237, 72, 51, 0, 197, 229, 178, 114, 41, 110,
        142, 168, 13, 217, 63, 133, 238, 215, 0, 100, 255, 128, 113, 228, 155, 194, 137, 51, 196,
        233, 154, 146, 156, 13, 32, 162, 1, 10, 176, 205, 181, 169, 236, 113, 7, 209,
    ];
    assert_eq!(
        input.len() * 8,
        CRHWindow::NUM_WINDOWS * CRHWindow::WINDOW_SIZE
    );

    // Primitive setup and evaluation.
    let parameters = setup_crh();
    let primitive_result = CRH::evaluate(&parameters, &input).unwrap();

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::<SW6Fr>::new();

    // Alloc input.
    let mut input_bytes = vec![];
    for (byte_i, input_byte) in input.iter().enumerate() {
        let cs = test_cs.ns(|| format!("input_byte_gadget_{}", byte_i));
        input_bytes.push(UInt8::alloc(cs, || Ok(*input_byte)).unwrap());
    }

    // Alloc parameters.
    let gadget_parameters =
        CRHGadgetParameters::alloc(test_cs.ns(|| "gadget_parameters"), || Ok(&parameters)).unwrap();

    // Evaluate on circuit.
    let gadget_result = CRHGadget::check_evaluation_gadget(
        test_cs.ns(|| "gadget_evaluation"),
        &gadget_parameters,
        &input_bytes,
    )
    .unwrap();

    let primitive_result = primitive_result.into_affine();
    assert_eq!(primitive_result.x, gadget_result.x.value.unwrap());
    assert_eq!(primitive_result.y, gadget_result.y.value.unwrap());
    assert!(test_cs.is_satisfied())
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Window;

impl PedersenWindow for Window {
    const WINDOW_SIZE: usize = 128;
    const NUM_WINDOWS: usize = 8;
}

type TestCRH = PedersenCRH<G1Projective, Window>;
type TestCRHGadget = PedersenCRHGadget<G1Projective, SW6Fr, G1Gadget>;

fn generate_input<CS: ConstraintSystem<SW6Fr>, R: Rng>(
    mut cs: CS,
    rng: &mut R,
) -> ([u8; 128], Vec<UInt8>) {
    let mut input = [1u8; 128];
    rng.fill_bytes(&mut input);

    let mut input_bytes = vec![];
    for (byte_i, input_byte) in input.iter().enumerate() {
        let cs = cs.ns(|| format!("input_byte_gadget_{}", byte_i));
        input_bytes.push(UInt8::alloc(cs, || Ok(*input_byte)).unwrap());
    }
    (input, input_bytes)
}

#[test]
fn crh_primitive_gadget_test() {
    let rng = &mut test_rng();
    let mut cs = TestConstraintSystem::<SW6Fr>::new();

    let (input, input_bytes) = generate_input(&mut cs, rng);
    println!("number of constraints for input: {}", cs.num_constraints());

    let parameters = TestCRH::setup(rng).unwrap();
    let primitive_result = TestCRH::evaluate(&parameters, &input).unwrap();

    let gadget_parameters =
        <TestCRHGadget as FixedLengthCRHGadget<TestCRH, SW6Fr>>::ParametersGadget::alloc(
            &mut cs.ns(|| "gadget_parameters"),
            || Ok(&parameters),
        )
        .unwrap();
    println!(
        "number of constraints for input + params: {}",
        cs.num_constraints()
    );

    let gadget_result =
        <TestCRHGadget as FixedLengthCRHGadget<TestCRH, SW6Fr>>::check_evaluation_gadget(
            &mut cs.ns(|| "gadget_evaluation"),
            &gadget_parameters,
            &input_bytes,
        )
        .unwrap();

    println!("number of constraints total: {}", cs.num_constraints());

    let primitive_result = primitive_result.into_affine();
    assert_eq!(primitive_result.x, gadget_result.x.value.unwrap());
    assert_eq!(primitive_result.y, gadget_result.y.value.unwrap());
    assert!(cs.is_satisfied());
}
