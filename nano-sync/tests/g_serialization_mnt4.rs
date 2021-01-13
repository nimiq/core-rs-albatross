// use rand::RngCore;
//
// use ark_mnt4_753::constraints::{G1Var, G2Var};
// use ark_mnt4_753::{FqParameters, Fr, G1Projective, G2Projective};
// use ark_mnt6_753::Fr as MNT6Fr;
// use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget, ToBitsGadget};
// use ark_r1cs_std::R1CSVar;
// use ark_relations::r1cs::{ConstraintSystem, ConstraintSystemRef};
// use ark_std::{test_rng, UniformRand};
// use nimiq_nano_sync::gadgets::mnt6::{SerializeGadget, YToBitGadget};
// use nimiq_nano_sync::primitives::{serialize_g1_mnt4, serialize_g2_mnt4};
// use nimiq_nano_sync::utils::{bytes_to_bits, pad_point_bits};
//
// // When running tests you are advised to run only one test at a time or you might run out of RAM.
// // Also they take a long time to run. This is why they have the ignore flag.
//
// #[test]
// //#[ignore]
// fn serialization_mnt4_works() {
//     // Initialize the constraint system.
//     let cs = ConstraintSystem::<MNT6Fr>::new_ref();
//
//     // Create random integer.
//     let rng = &mut test_rng();
//
//     // Create random points.
//     let g2_point = G2Projective::rand(rng);
//     let g1_point = G1Projective::rand(rng);
//
//     // Allocate the random inputs in the circuit.
//     let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point.clone())).unwrap();
//     let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point.clone())).unwrap();
//
//     // -----------  G1  -----------
//     // Serialize using the primitive version.
//     let bytes = serialize_g1_mnt4(g1_point);
//     let bits = bytes_to_bits(&bytes);
//
//     // Allocate the primitive result for easier comparison.
//     let mut primitive_bits = Vec::new();
//
//     for bit in bits {
//         primitive_bits.push(Boolean::<MNT6Fr>::new_witness(cs.clone(), || Ok(bit)).unwrap());
//     }
//
//     for i in 7..16 {
//         print!("{:?} ", &primitive_bits[i].value().unwrap());
//     }
//     print!(" - ");
//     for i in primitive_bits.len() - 8..primitive_bits.len() {
//         print!("{:?} ", &primitive_bits[i].value().unwrap());
//     }
//     println!();
//
//     // Serialize using the gadget version.
//     let mut gadget_bits = SerializeGadget::serialize_g1(cs.clone(), &g1_point_var).unwrap();
//
//     for i in 7..16 {
//         print!("{:?} ", &gadget_bits[i].value().unwrap());
//     }
//     print!(" - ");
//     for i in gadget_bits.len() - 8..gadget_bits.len() {
//         print!("{:?} ", &gadget_bits[i].value().unwrap());
//     }
//     println!();
//
//     assert_eq!(primitive_bits.len(), gadget_bits.len());
//
//     assert!(primitive_bits.is_eq(&gadget_bits).unwrap().value().unwrap());
//
//     // // -----------  G1  -----------
//     // // Serialize using the primitive version.
//     // let bytes = serialize_g1_mnt4(g1_point);
//     // let bits = bytes_to_bits(&bytes);
//     //
//     // // Allocate the primitive result for easier comparison.
//     // let mut primitive_var: Vec<Boolean> = Vec::new();
//     // for i in 0..bits.len() {
//     //     primitive_var.push(
//     //         Boolean::alloc(
//     //             cs.ns(|| format!("allocate primitive result g1: bit {}", i)),
//     //             || Ok(bits[i]),
//     //         )
//     //         .unwrap(),
//     //     );
//     // }
//     //
//     // // Serialize using the gadget version.
//     // let mut gadget_var = vec![];
//     // let x_bits = g1_point_var.x.to_bits(cs.ns(|| "g1 x to bits")).unwrap();
//     // let y_bit = YToBitGadget::y_to_bit_g1(cs.ns(|| "g1 y to bit"), &g1_point_var).unwrap();
//     // gadget_var.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
//     //
//     // assert_eq!(primitive_var, gadget_var);
// }
