use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::prelude::{AllocVar, Boolean};
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::ConstraintSystem;
use ark_std::test_rng;
use rand::RngCore;

use nimiq_nano_sync::gadgets::mnt4::MerkleTreeGadget;
use nimiq_nano_sync::primitives::{
    merkle_tree_construct, merkle_tree_prove, merkle_tree_verify, pedersen_generators,
    pedersen_hash, serialize_g1_mnt6,
};
use nimiq_nano_sync::utils::{byte_from_le_bits, bytes_to_bits};

#[test]
fn merkle_tree_construct_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bits.
    let mut bytes = [0u8; 128];
    let mut leaves = Vec::new();
    for _ in 0..16 {
        rng.fill_bytes(&mut bytes);
        leaves.push(bytes_to_bits(&bytes));
    }

    // Construct Merkle tree using the primitive version.
    let primitive_tree = bytes_to_bits(&merkle_tree_construct(leaves.clone()));

    // Allocate the random bits in the circuit.
    let mut leaves_var = vec![];
    let mut bits_var = vec![];
    for leaf in leaves {
        for bit in leaf {
            bits_var.push(Boolean::new_witness(cs.clone(), || Ok(bit)).unwrap());
        }
        leaves_var.push(bits_var.clone());
        bits_var.clear();
    }

    // Generate and allocate the Pedersen generators in the circuit.
    let generators = pedersen_generators(4);

    let mut pedersen_generators_var = vec![];
    for generator in generators {
        pedersen_generators_var.push(G1Var::new_witness(cs.clone(), || Ok(generator)).unwrap());
    }

    // Construct Merkle tree using the gadget version.
    let gadget_tree =
        MerkleTreeGadget::construct(cs.clone(), &leaves_var, &pedersen_generators_var).unwrap();

    // Compare the two versions bit by bit.
    assert_eq!(primitive_tree.len(), gadget_tree.len());
    for i in 0..primitive_tree.len() {
        assert_eq!(primitive_tree[i], gadget_tree[i].value().unwrap());
    }
}

#[test]
fn merkle_tree_prove_works() {
    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bits.
    let mut bytes = [0u8; 128];
    let mut leaves = Vec::new();
    for _ in 0..16 {
        rng.fill_bytes(&mut bytes);
        leaves.push(bytes_to_bits(&bytes));
    }

    // Create random position.
    let mut byte = [0u8; 1];
    rng.fill_bytes(&mut byte);
    let mut path = bytes_to_bits(&byte);
    path.truncate(4);
    let position = byte_from_le_bits(&path) as usize;

    // Calculate root.
    let root = merkle_tree_construct(leaves.clone());

    // Calculate proof.
    let proof = merkle_tree_prove(leaves.clone(), path.clone());

    // Verify proof.
    let input = leaves.get(position).unwrap().to_vec();

    assert!(merkle_tree_verify(input, proof, path, root))
}

#[test]
fn merkle_tree_verify_works() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bits.
    let mut bytes = [0u8; 128];
    rng.fill_bytes(&mut bytes);
    let leaf = bytes_to_bits(&bytes);

    // Create the Pedersen generators.
    let generators = pedersen_generators(4);

    // Create fake Merkle tree branch.
    let path = vec![false, true, false, true];
    let mut bytes = [0u8; 95];
    let mut nodes = vec![];
    let mut bits = vec![];

    let mut node = pedersen_hash(leaf.clone(), generators.clone());

    for i in 0..4 {
        rng.fill_bytes(&mut bytes);
        let other_node = pedersen_hash(bytes_to_bits(&bytes), generators.clone());

        if path[i] {
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
        } else {
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
        }

        nodes.push(other_node);
        node = pedersen_hash(bits.clone(), generators.clone());
        bits.clear();
    }

    // Create root.
    let root = serialize_g1_mnt6(node).to_vec();
    let root_bits = bytes_to_bits(&root);

    // Verify Merkle proof using the primitive version.
    assert!(merkle_tree_verify(
        leaf.clone(),
        nodes.clone(),
        path.clone(),
        root.clone(),
    ));

    // Allocate the leaf in the circuit.
    let leaf_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(leaf)).unwrap();

    // Allocate the nodes in the circuit.
    let nodes_var = Vec::<G1Var>::new_witness(cs.clone(), || Ok(nodes)).unwrap();

    // Allocate the path in the circuit.
    let path_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(path)).unwrap();

    // Allocate the root in the circuit.
    let root_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(root_bits)).unwrap();

    // Allocate the Pedersen generators in the circuit.
    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(4))).unwrap();

    // Verify Merkle proof.
    assert!(MerkleTreeGadget::verify(
        cs.clone(),
        &leaf_var,
        &nodes_var,
        &path_var,
        &root_var,
        &generators_var,
    )
    .unwrap()
    .value()
    .unwrap())
}

#[test]
fn merkle_tree_verify_wrong_root() {
    // Initialize the constraint system.
    let cs = ConstraintSystem::<MNT4Fr>::new_ref();

    // Create random number generator.
    let rng = &mut test_rng();

    // Create random bits.
    let mut bytes = [0u8; 128];
    rng.fill_bytes(&mut bytes);
    let leaf = bytes_to_bits(&bytes);

    // Create the Pedersen generators.
    let generators = pedersen_generators(4);

    // Create fake Merkle tree branch.
    let path = vec![false, true, false, true];
    let mut bytes = [0u8; 95];
    let mut nodes = vec![];
    let mut bits = vec![];

    let mut node = pedersen_hash(leaf.clone(), generators.clone());

    for i in 0..4 {
        rng.fill_bytes(&mut bytes);
        let other_node = pedersen_hash(bytes_to_bits(&bytes), generators.clone());

        if path[i] {
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
        } else {
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
            bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref());
        }

        nodes.push(other_node);
        node = pedersen_hash(bits.clone(), generators.clone());
        bits.clear();
    }

    // Create wrong root.
    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let root = bytes.to_vec();
    let root_bits = bytes_to_bits(&root);

    // Verify Merkle proof using the primitive version.
    assert!(!merkle_tree_verify(
        leaf.clone(),
        nodes.clone(),
        path.clone(),
        root.clone(),
    ));

    // Allocate the leaf in the circuit.
    let leaf_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(leaf)).unwrap();

    // Allocate the nodes in the circuit.
    let nodes_var = Vec::<G1Var>::new_witness(cs.clone(), || Ok(nodes)).unwrap();

    // Allocate the path in the circuit.
    let path_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(path)).unwrap();

    // Allocate the root in the circuit.
    let root_var = Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(root_bits)).unwrap();

    // Allocate the Pedersen generators in the circuit.
    let generators_var =
        Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(4))).unwrap();

    // Verify Merkle proof.
    assert!(!MerkleTreeGadget::verify(
        cs.clone(),
        &leaf_var,
        &nodes_var,
        &path_var,
        &root_var,
        &generators_var,
    )
    .unwrap()
    .value()
    .unwrap())
}
