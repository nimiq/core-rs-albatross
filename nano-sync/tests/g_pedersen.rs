// use rand::RngCore;
//
// use algebra::mnt4_753::Fr as MNT4Fr;
// use algebra::test_rng;
// use nimiq_nano_sync::gadgets::mnt4::PedersenHashGadget;
// use nimiq_nano_sync::primitives::{pedersen_generators, pedersen_hash};
// use nimiq_nano_sync::utils::bytes_to_bits;
// use r1cs_core::ConstraintSystem;
// use r1cs_std::bits::boolean::Boolean;
// use r1cs_std::mnt6_753::G1Gadget;
// use r1cs_std::prelude::AllocGadget;
// use r1cs_std::test_constraint_system::TestConstraintSystem;
//
// // When running tests you are advised to run only one test at a time or you might run out of RAM.
// // Also they take a long time to run. This is why they have the ignore flag.
//
// #[test]
// #[ignore]
// fn pedersen_hash_works() {
//     // Initialize the constraint system.
//     let mut cs = TestConstraintSystem::<MNT4Fr>::new();
//
//     // Create random bits.
//     let rng = &mut test_rng();
//     let mut bytes = [0u8; 450];
//     rng.fill_bytes(&mut bytes);
//     let bits = bytes_to_bits(&bytes);
//
//     // Generate the generators for the Pedersen hash.
//     let generators = pedersen_generators(6);
//
//     // Evaluate Pedersen hash using the primitive version.
//     let primitive_out = pedersen_hash(bits.clone(), generators.clone());
//
//     // Convert the result to a G1Gadget for easier comparison.
//     let primitive_out_var =
//         G1Gadget::alloc(cs.ns(|| "allocate primitive result"), || Ok(primitive_out)).unwrap();
//
//     // Allocate the random bits in the circuit.
//     let mut bits_var = vec![];
//     for i in 0..3600 {
//         bits_var.push(
//             Boolean::alloc(cs.ns(|| format!("allocating input bit {}", i)), || {
//                 Ok(&bits[i])
//             })
//             .unwrap(),
//         );
//     }
//
//     // Allocate the Pedersen generators in the circuit.
//     let mut c_generators = Vec::new();
//     for i in 0..generators.len() {
//         let base = G1Gadget::alloc(
//             &mut cs.ns(|| format!("allocating pedersen generator {}", i)),
//             || Ok(&generators[i]),
//         )
//         .unwrap();
//         c_generators.push(base);
//     }
//
//     // Evaluate Pedersen hash using the gadget version.
//     let gadget_out = PedersenHashGadget::evaluate(
//         cs.ns(|| "evaluate pedersen gadget"),
//         &bits_var,
//         &c_generators,
//     )
//     .unwrap();
//
//     assert_eq!(primitive_out_var, gadget_out)
// }
