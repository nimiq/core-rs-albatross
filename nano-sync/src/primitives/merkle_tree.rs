use std::cmp;

use algebra::mnt6_753::G1Projective;

use crate::constants::sum_generator_g1_mnt6;
use crate::primitives::{pedersen_commitment, pedersen_generators};
use crate::utils::{byte_from_le_bits, bytes_to_bits, serialize_g1_mnt6};

/// Creates a Merkle tree from the given inputs, as a vector of vectors of booleans, and outputs
/// the root. Each vector of booleans is meant to be one leaf. Each leaf can be of a different
/// size. Number of leaves has to be a power of two.
/// The tree is constructed from left to right. For example, if we are given inputs {0, 1, 2, 3}
/// then the resulting tree will be:
///                      o
///                    /   \
///                   o     o
///                  / \   / \
///                 0  1  2  3
pub fn merkle_tree_construct(inputs: Vec<Vec<bool>>) -> Vec<u8> {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert_eq!((inputs.len() & (inputs.len() - 1)), 0);

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let mut generators_needed = 3; // At least this much is required for the non-leaf nodes.

    for i in 0..inputs.len() {
        generators_needed = cmp::max(generators_needed, (inputs[i].len() + 752 - 1) / 752);
    }

    let generators = pedersen_generators(generators_needed);

    let sum_generator = sum_generator_g1_mnt6();

    // Calculate the Pedersen commitments for the leaves.
    let mut nodes = Vec::new();

    for input in inputs {
        let pedersen_commitment = pedersen_commitment(input, generators.clone(), sum_generator);
        nodes.push(pedersen_commitment);
    }

    // Calculate the rest of the tree
    let mut next_nodes = Vec::new();

    while nodes.len() > 1 {
        // Process each level of nodes.
        for j in 0..nodes.len() / 2 {
            let mut bytes = Vec::new();

            // Serialize the left node.
            bytes.extend_from_slice(serialize_g1_mnt6(nodes[2 * j]).as_ref());

            // Serialize the right node.
            bytes.extend_from_slice(serialize_g1_mnt6(nodes[2 * j + 1]).as_ref());

            // Calculate the parent node.
            let bits = bytes_to_bits(&bytes);
            let parent_node = pedersen_commitment(bits, generators.clone(), sum_generator);

            next_nodes.push(parent_node);
        }
        nodes.clear();

        nodes.append(&mut next_nodes);
    }

    // Serialize the root node.
    let bytes = serialize_g1_mnt6(nodes[0]);

    Vec::from(bytes.as_ref())
}

/// Verifies a Merkle proof. More specifically, given an input and all of the tree nodes up to
/// the root, it checks if the input is part of the Merkle tree or not. The path is simply the
/// position of the input leaf in little-endian binary. For example, for the given tree:
///                      o
///                    /   \
///                   o     o
///                  / \   / \
///                 0  1  2  3
/// The path for the leaf 2 is simply 01. Another way of thinking about it is that if you go up
/// the tree, each time you are the left node it's a zero and if you are the right node it's an
/// one.
pub fn merkle_tree_verify(
    input: Vec<bool>,
    nodes: Vec<G1Projective>,
    path: Vec<bool>,
    root: Vec<u8>,
) -> bool {
    // Checking that the inputs vector is not empty.
    assert!(!input.is_empty());

    // Checking that the nodes vector is not empty.
    assert!(!nodes.is_empty());

    // Checking that there is one node for each path bit.
    assert_eq!(nodes.len(), path.len());

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let generators_needed = cmp::max(3, (input.len() + 752 - 1) / 752);

    let generators = pedersen_generators(generators_needed);

    let sum_generator = sum_generator_g1_mnt6();

    // Calculate the Pedersen commitments for the input.
    let mut result = pedersen_commitment(input, generators.clone(), sum_generator);

    // Calculate the root of the tree using the branch values.
    let mut left_node;

    let mut right_node;

    for i in 0..nodes.len() {
        // Decide which node is the left or the right one based on the path.
        if path[i] {
            left_node = nodes[i];
            right_node = result;
        } else {
            left_node = result;
            right_node = nodes[i];
        }

        // Serialize the left and right nodes.
        let mut bytes = Vec::new();

        bytes.extend_from_slice(serialize_g1_mnt6(left_node).as_ref());

        bytes.extend_from_slice(serialize_g1_mnt6(right_node).as_ref());

        let bits = bytes_to_bits(&bytes);

        // Calculate the parent node and update result.
        result = pedersen_commitment(bits, generators.clone(), sum_generator);
    }

    // Serialize the root node.
    let bytes = serialize_g1_mnt6(result);

    let reference = Vec::from(bytes.as_ref());

    // Check if the calculated root is equal to the given root.
    root == reference
}

/// Creates a Merkle proof given all the leaf nodes and the path to the node for which we want the
/// proof. Basically, it just constructs the whole tree while storing all the intermediate nodes
/// needed for the Merkle proof. It does not output either the root or the input leaf node since
/// these are assumed to be already known bu the verifier.
/// The path is simply the position of the input leaf in little-endian binary. For example, for the
/// given tree:
///                      o
///                    /   \
///                   o     o
///                  / \   / \
///                 0  1  2  3
/// The path for the leaf 2 is simply 01. Another way of thinking about it is that if you go up
/// the tree, each time you are the left node it's a zero and if you are the right node it's an
/// one.
pub fn merkle_tree_prove(inputs: Vec<Vec<bool>>, path: Vec<bool>) -> Vec<G1Projective> {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert_eq!((inputs.len() & (inputs.len() - 1)), 0);

    // Check that the path is of the right size.
    assert_eq!(2_u32.pow(path.len() as u32), inputs.len() as u32);

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let mut generators_needed = 3; // At least this much is required for the non-leaf nodes.

    for i in 0..inputs.len() {
        generators_needed = cmp::max(generators_needed, (inputs[i].len() + 752 - 1) / 752);
    }

    let generators = pedersen_generators(generators_needed);

    let sum_generator = sum_generator_g1_mnt6();

    // Calculate the Pedersen commitments for the leaves.
    let mut nodes = Vec::new();

    for input in inputs {
        let pedersen_commitment = pedersen_commitment(input, generators.clone(), sum_generator);
        nodes.push(pedersen_commitment);
    }

    // Calculate the rest of the tree
    let mut next_nodes = Vec::new();

    let mut proof = Vec::new();

    let mut i = 0;

    while nodes.len() > 1 {
        // Calculate the position of the node needed for the proof.
        let proof_position = byte_from_le_bits(&path[i..]) as usize;

        // Process each level of nodes.
        for j in 0..nodes.len() / 2 {
            let mut bytes = Vec::new();

            // Store the proof node, if applicable.
            if proof_position == 2 * j {
                proof.push(nodes[2 * j + 1]);
            }

            if proof_position == 2 * j + 1 {
                proof.push(nodes[2 * j]);
            }

            // Serialize the left node.
            bytes.extend_from_slice(serialize_g1_mnt6(nodes[2 * j]).as_ref());

            // Serialize the right node.
            bytes.extend_from_slice(serialize_g1_mnt6(nodes[2 * j + 1]).as_ref());

            // Calculate the parent node.
            let bits = bytes_to_bits(&bytes);
            let parent_node = pedersen_commitment(bits, generators.clone(), sum_generator);

            next_nodes.push(parent_node.clone());
        }
        nodes.clear();

        nodes.append(&mut next_nodes);

        i += 1;
    }

    proof
}
