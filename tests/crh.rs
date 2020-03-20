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

// #[test]
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
