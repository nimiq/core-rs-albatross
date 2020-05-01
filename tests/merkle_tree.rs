use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::test_rng;
use r1cs_core::ConstraintSystem;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::mnt6_753::G1Gadget;
use r1cs_std::prelude::{AllocGadget, UInt8};
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::constants::sum_generator_g1_mnt6;
use nano_sync::gadgets::mnt4::MerkleTreeGadget;
use nano_sync::primitives::{
    merkle_tree_construct, merkle_tree_verify, pedersen_commitment, pedersen_generators,
};
use nano_sync::utils::{bytes_to_bits, serialize_g1_mnt6};

//#[test]
fn merkle_tree_construct_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random bits.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 128];
    let mut leaves = Vec::new();
    for _ in 0..16 {
        rng.fill_bytes(&mut bytes);
        leaves.push(bytes_to_bits(&bytes));
    }

    // Construct Merkle tree using the primitive version.
    let primitive_out = merkle_tree_construct(leaves.clone());

    // Convert the result to a UInt8 for easier comparison.
    let mut primitive_out_var: Vec<UInt8> = Vec::new();
    for i in 0..primitive_out.len() {
        primitive_out_var.push(
            UInt8::alloc(
                cs.ns(|| format!("allocate primitive result: chunk {}", i)),
                || Ok(primitive_out[i]),
            )
            .unwrap(),
        );
    }

    // Allocate the random bits in the circuit.
    let mut leaves_var = Vec::new();
    let mut bits_var = Vec::new();
    let mut j = 0;
    for leaf in leaves {
        for i in 0..leaf.len() {
            bits_var.push(
                Boolean::alloc(
                    cs.ns(|| format!("allocating input bit {} {}", j, i)),
                    || Ok(&leaf[i]),
                )
                .unwrap(),
            );
        }
        leaves_var.push(bits_var.clone());
        bits_var.clear();
        j += 1;
    }

    // Generate and allocate the Pedersen generators in the circuit.
    let generators = pedersen_generators(3);
    let mut c_generators = Vec::new();
    for i in 0..generators.len() {
        let base = G1Gadget::alloc(
            &mut cs.ns(|| format!("allocating pedersen generator {}", i)),
            || Ok(&generators[i]),
        )
        .unwrap();
        c_generators.push(base);
    }

    // Allocate the sum generator in the circuit.
    let sum_generator = G1Gadget::alloc(cs.ns(|| "allocating sum generator"), || {
        Ok(sum_generator_g1_mnt6())
    })
    .unwrap();

    // Construct Merkle tree using the gadget version.
    let gadget_out = MerkleTreeGadget::construct(
        cs.ns(|| "evaluate pedersen gadget"),
        &leaves_var,
        &c_generators,
        &sum_generator,
    )
    .unwrap();

    assert_eq!(primitive_out_var, gadget_out)
}

//#[test]
fn merkle_tree_verify_works() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random bits.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 128];
    rng.fill_bytes(&mut bytes);
    let leaf = bytes_to_bits(&bytes);

    // Generate the generators for the Pedersen commitment.
    let generators = pedersen_generators(3);
    let sum_generator = sum_generator_g1_mnt6();

    // Create fake Merkle tree branch.
    let path = vec![false, true, false, true];
    let mut bytes = [0u8; 95];
    let mut nodes = Vec::new();
    let mut bits = Vec::new();
    let mut node = pedersen_commitment(leaf.clone(), generators.clone(), sum_generator.clone());
    for i in 0..4 {
        rng.fill_bytes(&mut bytes);
        let other_node = pedersen_commitment(
            bytes_to_bits(&bytes),
            generators.clone(),
            sum_generator_g1_mnt6(),
        );
        if path[i] {
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(other_node.clone()).as_ref()).as_ref(),
            );
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(node.clone()).as_ref()).as_ref(),
            );
        } else {
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(node.clone()).as_ref()).as_ref(),
            );
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(other_node.clone()).as_ref()).as_ref(),
            );
        }
        nodes.push(other_node);
        node = pedersen_commitment(bits.clone(), generators.clone(), sum_generator.clone());
        bits.clear();
    }
    let root = serialize_g1_mnt6(node).to_vec();

    // Verify Merkle proof using the primitive version.
    merkle_tree_verify(leaf.clone(), nodes.clone(), path.clone(), root.clone());

    // Allocate the input in the circuit.
    let mut leaf_var = Vec::new();
    for i in 0..leaf.len() {
        leaf_var.push(
            Boolean::alloc(cs.ns(|| format!("allocating leaf bit {}", i)), || {
                Ok(&leaf[i])
            })
            .unwrap(),
        );
    }

    // Allocate the nodes in the circuit.
    let mut nodes_var = Vec::new();
    for i in 0..nodes.len() {
        nodes_var.push(
            G1Gadget::alloc(&mut cs.ns(|| format!("allocating node {}", i)), || {
                Ok(&nodes[i])
            })
            .unwrap(),
        );
    }

    // Allocate the path in the circuit.
    let mut path_var = Vec::new();
    for i in 0..path.len() {
        path_var.push(
            Boolean::alloc(cs.ns(|| format!("allocating path bit {}", i)), || {
                Ok(&path[i])
            })
            .unwrap(),
        );
    }

    // Allocate the root in the circuit.
    let mut root_var = Vec::new();
    for i in 0..root.len() {
        root_var.push(
            UInt8::alloc(cs.ns(|| format!("allocating root byte {}", i)), || {
                Ok(&root[i])
            })
            .unwrap(),
        );
    }

    // Allocate the Pedersen generators in the circuit.
    let mut generators_var = Vec::new();
    for i in 0..generators.len() {
        generators_var.push(
            G1Gadget::alloc(
                &mut cs.ns(|| format!("allocating pedersen generator {}", i)),
                || Ok(&generators[i]),
            )
            .unwrap(),
        );
    }

    // Allocate the sum generator in the circuit.
    let sum_generator_var =
        G1Gadget::alloc(cs.ns(|| "allocating sum generator"), || Ok(sum_generator)).unwrap();

    // Verify Merkle proof using the gadget version.
    MerkleTreeGadget::verify(
        cs.ns(|| "verify merkle proof"),
        &leaf_var,
        &nodes_var,
        &path_var,
        &root_var,
        &generators_var,
        &sum_generator_var,
    )
    .unwrap();

    assert!(cs.is_satisfied())
}

#[test]
fn wrong_root() {
    // Initialize the constraint system.
    let mut cs = TestConstraintSystem::<MNT4Fr>::new();

    // Create random bits.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 128];
    rng.fill_bytes(&mut bytes);
    let leaf = bytes_to_bits(&bytes);

    // Generate the generators for the Pedersen commitment.
    let generators = pedersen_generators(3);
    let sum_generator = sum_generator_g1_mnt6();

    // Create fake Merkle tree branch.
    let path = vec![false, true, false, true];
    let mut bytes = [0u8; 95];
    let mut nodes = Vec::new();
    let mut bits = Vec::new();
    let mut node = pedersen_commitment(leaf.clone(), generators.clone(), sum_generator.clone());
    for i in 0..4 {
        rng.fill_bytes(&mut bytes);
        let other_node = pedersen_commitment(
            bytes_to_bits(&bytes),
            generators.clone(),
            sum_generator_g1_mnt6(),
        );
        if path[i] {
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(other_node.clone()).as_ref()).as_ref(),
            );
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(node.clone()).as_ref()).as_ref(),
            );
        } else {
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(node.clone()).as_ref()).as_ref(),
            );
            bits.extend_from_slice(
                bytes_to_bits(serialize_g1_mnt6(other_node.clone()).as_ref()).as_ref(),
            );
        }
        nodes.push(other_node);
        node = pedersen_commitment(bits.clone(), generators.clone(), sum_generator.clone());
        bits.clear();
    }
    let root = serialize_g1_mnt6(node).to_vec();

    // Verify Merkle proof using the primitive version.
    //merkle_tree_verify(leaf.clone(), nodes.clone(), path.clone(), vec![0u8; 95]);

    // Allocate the input in the circuit.
    let mut leaf_var = Vec::new();
    for i in 0..leaf.len() {
        leaf_var.push(
            Boolean::alloc(cs.ns(|| format!("allocating leaf bit {}", i)), || {
                Ok(&leaf[i])
            })
            .unwrap(),
        );
    }

    // Allocate the nodes in the circuit.
    let mut nodes_var = Vec::new();
    for i in 0..nodes.len() {
        nodes_var.push(
            G1Gadget::alloc(&mut cs.ns(|| format!("allocating node {}", i)), || {
                Ok(&nodes[i])
            })
            .unwrap(),
        );
    }

    // Allocate the path in the circuit.
    let mut path_var = Vec::new();
    for i in 0..path.len() {
        path_var.push(
            Boolean::alloc(cs.ns(|| format!("allocating path bit {}", i)), || {
                Ok(&path[i])
            })
            .unwrap(),
        );
    }

    // Allocate a wrong root in the circuit.
    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let mut root_var = Vec::new();
    for i in 0..bytes.len() {
        root_var.push(
            UInt8::alloc(cs.ns(|| format!("allocating root byte {}", i)), || {
                Ok(&bytes[i])
            })
            .unwrap(),
        );
    }

    // Allocate the Pedersen generators in the circuit.
    let mut generators_var = Vec::new();
    for i in 0..generators.len() {
        generators_var.push(
            G1Gadget::alloc(
                &mut cs.ns(|| format!("allocating pedersen generator {}", i)),
                || Ok(&generators[i]),
            )
            .unwrap(),
        );
    }

    // Allocate the sum generator in the circuit.
    let sum_generator_var =
        G1Gadget::alloc(cs.ns(|| "allocating sum generator"), || Ok(sum_generator)).unwrap();

    // Verify Merkle proof using the gadget version.
    let result = MerkleTreeGadget::verify(
        cs.ns(|| "verify merkle proof"),
        &leaf_var,
        &nodes_var,
        &path_var,
        &root_var,
        &generators_var,
        &sum_generator_var,
    );

    assert!(result.is_err())
}
