// use rand::RngCore;
//
// use algebra::mnt4_753::Fr as MNT4Fr;
// use algebra::mnt6_753::G1Projective;
// use algebra::test_rng;
// use nimiq_nano_sync::gadgets::mnt4::MerkleTreeGadget;
// use nimiq_nano_sync::primitives::{
//     merkle_tree_construct, merkle_tree_prove, merkle_tree_verify, pedersen_generators,
//     pedersen_hash,
// };
// use nimiq_nano_sync::utils::{byte_from_le_bits, bytes_to_bits, serialize_g1_mnt6};
// use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
// use r1cs_std::bits::boolean::Boolean;
// use r1cs_std::mnt6_753::G1Gadget;
// use r1cs_std::prelude::{AllocGadget, UInt8};
// use r1cs_std::test_constraint_system::TestConstraintSystem;
//
// // When running tests you are advised to run only one test at a time or you might run out of RAM.
// // Also they take a long time to run. This is why they have the ignore flag.
//
// #[derive(Clone)]
// struct VerifyCircuit {
//     leaf: Vec<bool>,
//     nodes: Vec<G1Projective>,
//     path: Vec<bool>,
//     root: Vec<u8>,
// }
//
// impl VerifyCircuit {
//     pub fn new(leaf: Vec<bool>, nodes: Vec<G1Projective>, path: Vec<bool>, root: Vec<u8>) -> Self {
//         Self {
//             leaf,
//             nodes,
//             path,
//             root,
//         }
//     }
// }
//
// impl ConstraintSynthesizer<MNT4Fr> for VerifyCircuit {
//     fn generate_constraints<CS: ConstraintSystem<MNT4Fr>>(
//         self,
//         cs: &mut CS,
//     ) -> Result<(), SynthesisError> {
//         // Allocate the input in the circuit.
//         let leaf_var = Vec::<Boolean>::alloc(cs.ns(|| "alloc leaf"), || Ok(self.leaf.as_ref()))?;
//
//         // Allocate the nodes in the circuit.
//         let nodes_var =
//             Vec::<G1Gadget>::alloc(cs.ns(|| "alloc nodes"), || Ok(self.nodes.as_ref()))?;
//
//         // Allocate the path in the circuit.
//         let path_var = Vec::<Boolean>::alloc(cs.ns(|| "alloc path"), || Ok(self.path.as_ref()))?;
//
//         // Allocate the root in the circuit.
//         let root_var = UInt8::alloc_vec(cs.ns(|| "alloc root"), &self.root)?;
//
//         // Allocate the Pedersen generators in the circuit.
//         let generators_var = Vec::<G1Gadget>::alloc_constant(
//             cs.ns(|| "alloc pedersen_generators"),
//             pedersen_generators(4),
//         )?;
//
//         // Verify Merkle proof.
//         MerkleTreeGadget::verify(
//             cs.ns(|| "verify merkle proof"),
//             &leaf_var,
//             &nodes_var,
//             &path_var,
//             &root_var,
//             &generators_var,
//         )
//     }
// }
//
// #[test]
// #[ignore]
// fn construct_works() {
//     // Initialize the constraint system.
//     let mut cs = TestConstraintSystem::<MNT4Fr>::new();
//
//     // Create random bits.
//     let rng = &mut test_rng();
//     let mut bytes = [0u8; 128];
//     let mut leaves = Vec::new();
//     for _ in 0..16 {
//         rng.fill_bytes(&mut bytes);
//         leaves.push(bytes_to_bits(&bytes));
//     }
//
//     // Construct Merkle tree using the primitive version.
//     let primitive_out = merkle_tree_construct(leaves.clone());
//
//     // Convert the result to a UInt8 for easier comparison.
//     let mut primitive_out_var: Vec<UInt8> = Vec::new();
//     for i in 0..primitive_out.len() {
//         primitive_out_var.push(
//             UInt8::alloc(
//                 cs.ns(|| format!("allocate primitive result: chunk {}", i)),
//                 || Ok(primitive_out[i]),
//             )
//             .unwrap(),
//         );
//     }
//
//     // Allocate the random bits in the circuit.
//     let mut leaves_var = Vec::new();
//     let mut bits_var = Vec::new();
//     let mut j = 0;
//     for leaf in leaves {
//         for i in 0..leaf.len() {
//             bits_var.push(
//                 Boolean::alloc(
//                     cs.ns(|| format!("allocating input bit {} {}", j, i)),
//                     || Ok(&leaf[i]),
//                 )
//                 .unwrap(),
//             );
//         }
//         leaves_var.push(bits_var.clone());
//         bits_var.clear();
//         j += 1;
//     }
//
//     // Generate and allocate the Pedersen generators in the circuit.
//     let generators = pedersen_generators(4);
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
//     // Construct Merkle tree using the gadget version.
//     let gadget_out = MerkleTreeGadget::construct(
//         cs.ns(|| "evaluate pedersen gadget"),
//         &leaves_var,
//         &c_generators,
//     )
//     .unwrap();
//
//     assert_eq!(primitive_out_var, gadget_out)
// }
//
// #[test]
// #[ignore]
// fn prove_works() {
//     // Create random bits.
//     let rng = &mut test_rng();
//     let mut bytes = [0u8; 128];
//     let mut leaves = Vec::new();
//     for _ in 0..16 {
//         rng.fill_bytes(&mut bytes);
//         leaves.push(bytes_to_bits(&bytes));
//     }
//     let mut byte = [0u8; 1];
//     rng.fill_bytes(&mut byte);
//     let mut path = bytes_to_bits(&byte);
//     path.truncate(4);
//     let position = byte_from_le_bits(&path) as usize;
//
//     // Calculate root.
//     let root = merkle_tree_construct(leaves.clone());
//
//     // Calculate proof.
//     let proof = merkle_tree_prove(leaves.clone(), path.clone());
//
//     // Verify proof.
//     let input = leaves.get(position).unwrap().to_vec();
//
//     assert!(merkle_tree_verify(input, proof, path, root))
// }
//
// #[test]
// #[ignore]
// fn verify_works() {
//     // Create random bits.
//     let rng = &mut test_rng();
//     let mut bytes = [0u8; 128];
//     rng.fill_bytes(&mut bytes);
//     let leaf = bytes_to_bits(&bytes);
//
//     // Generate the generators for the Pedersen hash.
//     let generators = pedersen_generators(4);
//
//     // Create fake Merkle tree branch.
//     let path = vec![false, true, false, true];
//     let mut bytes = [0u8; 95];
//     let mut nodes = Vec::new();
//     let mut bits = Vec::new();
//
//     let mut node = pedersen_hash(leaf.clone(), generators.clone());
//
//     for i in 0..4 {
//         rng.fill_bytes(&mut bytes);
//         let other_node = pedersen_hash(bytes_to_bits(&bytes), generators.clone());
//
//         if path[i] {
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
//         } else {
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
//         }
//
//         nodes.push(other_node);
//         node = pedersen_hash(bits.clone(), generators.clone());
//         bits.clear();
//     }
//
//     let root = serialize_g1_mnt6(node).to_vec();
//
//     // Verify Merkle proof using the primitive version.
//     assert!(merkle_tree_verify(
//         leaf.clone(),
//         nodes.clone(),
//         path.clone(),
//         root.clone(),
//     ));
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let circuit = VerifyCircuit::new(leaf, nodes, path, root);
//     circuit.generate_constraints(&mut test_cs).unwrap();
//
//     if !test_cs.is_satisfied() {
//         println!("Unsatisfied @ {}", test_cs.which_is_unsatisfied().unwrap());
//         assert!(false);
//     }
// }
//
// #[test]
// #[ignore]
// fn verify_wrong_root() {
//     // Create random bits.
//     let rng = &mut test_rng();
//     let mut bytes = [0u8; 128];
//     rng.fill_bytes(&mut bytes);
//     let leaf = bytes_to_bits(&bytes);
//
//     // Generate the generators for the Pedersen hash.
//     let generators = pedersen_generators(4);
//
//     // Create fake Merkle tree branch.
//     let path = vec![false, true, false, true];
//     let mut bytes = [0u8; 95];
//     let mut nodes = Vec::new();
//     let mut bits = Vec::new();
//
//     let mut node = pedersen_hash(leaf.clone(), generators.clone());
//
//     for i in 0..4 {
//         rng.fill_bytes(&mut bytes);
//         let other_node = pedersen_hash(bytes_to_bits(&bytes), generators.clone());
//
//         if path[i] {
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
//         } else {
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
//             bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
//         }
//
//         nodes.push(other_node);
//         node = pedersen_hash(bits.clone(), generators.clone());
//         bits.clear();
//     }
//
//     // Create wrong root.
//     let mut bytes = [0u8; 95];
//     rng.fill_bytes(&mut bytes);
//     let root = bytes.to_vec();
//
//     // Verify Merkle proof using the primitive version.
//     assert!(!merkle_tree_verify(
//         leaf.clone(),
//         nodes.clone(),
//         path.clone(),
//         root.clone(),
//     ));
//
//     // Test constraint system.
//     let mut test_cs = TestConstraintSystem::new();
//     let circuit = VerifyCircuit::new(leaf, nodes, path, root);
//     circuit.generate_constraints(&mut test_cs).unwrap();
//
//     assert!(!test_cs.is_satisfied());
// }
