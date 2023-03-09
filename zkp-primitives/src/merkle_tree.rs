use std::cmp;

use ark_mnt6_753::G1Projective;
#[cfg(feature = "parallel")]
use rayon::{
    iter::{IntoParallelRefIterator, ParallelIterator},
    prelude::IntoParallelIterator,
};

use crate::{pedersen::default_pedersen_hash, serialize::serialize_g1_mnt6, PEDERSEN_PARAMETERS};

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
pub fn merkle_tree_construct(inputs: Vec<Vec<u8>>) -> [u8; 95] {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert!(inputs.len().is_power_of_two());

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = 752;

    let mut generators_needed = 4; // At least this much is required for the non-leaf nodes.

    for input in &inputs {
        generators_needed = cmp::max(
            generators_needed,
            (input.len() + capacity - 1) / capacity + 1,
        );
    }

    assert!(
        generators_needed <= PEDERSEN_PARAMETERS.parameters.generators.len(),
        "Invalid number of pedersen generators"
    );

    // Calculate the Pedersen hashes for the leaves.
    #[cfg(not(feature = "parallel"))]
    let iter = inputs.into_iter();
    #[cfg(feature = "parallel")]
    let iter = inputs.into_par_iter();

    let mut nodes: Vec<G1Projective> = iter.map(|bytes| default_pedersen_hash(&bytes)).collect();

    // Process each level of nodes.
    while nodes.len() > 1 {
        #[cfg(not(feature = "parallel"))]
        let iter = nodes.iter();
        #[cfg(feature = "parallel")]
        let iter = nodes.par_iter();

        // Serialize all the child nodes.
        let bytes: Vec<u8> = iter
            .map(|node| serialize_g1_mnt6(node).to_vec())
            .flatten()
            .collect();

        // Chunk the bits into the number of parent nodes.
        let mut chunks = Vec::new();

        let num_chunks = nodes.len() / 2;

        for i in 0..num_chunks {
            chunks.push(
                bytes[i * bytes.len() / num_chunks..(i + 1) * bytes.len() / num_chunks].to_vec(),
            );
        }

        // Calculate the parent nodes.
        #[cfg(not(feature = "parallel"))]
        let iter = chunks.into_iter();
        #[cfg(feature = "parallel")]
        let iter = chunks.into_par_iter();

        let mut next_nodes: Vec<G1Projective> =
            iter.map(|bytes| default_pedersen_hash(&bytes)).collect();

        // Clear the child nodes and add the parent nodes.
        nodes.clear();
        nodes.append(&mut next_nodes);
    }

    // Serialize the root node.
    serialize_g1_mnt6(&nodes[0])
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
    input: &[u8],
    nodes: Vec<G1Projective>,
    path: Vec<bool>,
    root: [u8; 95],
) -> bool {
    // Checking that the inputs vector is not empty.
    assert!(!input.is_empty());

    // Checking that the nodes vector is not empty.
    assert!(!nodes.is_empty());

    // Checking that there is one node for each path bit.
    assert_eq!(nodes.len(), path.len());

    // Calculate the Pedersen hashes for the input.
    let mut result = default_pedersen_hash(input);

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

        bytes.extend_from_slice(serialize_g1_mnt6(&left_node).as_ref());

        bytes.extend_from_slice(serialize_g1_mnt6(&right_node).as_ref());

        // Calculate the parent node and update result.
        result = default_pedersen_hash(&bytes);
    }

    // Serialize the root node.
    let reference = serialize_g1_mnt6(&result);

    // Check if the calculated root is equal to the given root.
    root == reference
}

/// Creates a Merkle proof given all the leaf nodes and the path to the node for which we want the
/// proof. Basically, it just constructs the whole tree while storing all the intermediate nodes
/// needed for the Merkle proof. It does not output either the root or the input leaf node since
/// these are assumed to be already known by the verifier.
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
pub fn merkle_tree_prove(inputs: Vec<Vec<u8>>, path: Vec<bool>) -> Vec<G1Projective> {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert_eq!((inputs.len() & (inputs.len() - 1)), 0);

    // Check that the path is of the right size.
    assert_eq!(2_u32.pow(path.len() as u32), inputs.len() as u32);

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = 752;

    let mut generators_needed = 4; // At least this much is required for the non-leaf nodes.

    for input in &inputs {
        generators_needed = cmp::max(
            generators_needed,
            (input.len() + capacity - 1) / capacity + 1,
        );
    }

    // Calculate the Pedersen hashes for the leaves.
    let mut nodes = Vec::new();

    for input in inputs {
        let hash = default_pedersen_hash(&input);
        nodes.push(hash);
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
            bytes.extend_from_slice(serialize_g1_mnt6(&nodes[2 * j]).as_ref());

            // Serialize the right node.
            bytes.extend_from_slice(serialize_g1_mnt6(&nodes[2 * j + 1]).as_ref());

            // Calculate the parent node.
            let parent_node = default_pedersen_hash(&bytes);

            next_nodes.push(parent_node);
        }
        nodes.clear();

        nodes.append(&mut next_nodes);

        i += 1;
    }

    proof
}

/// Transforms a vector of little endian bits into a u8.
fn byte_from_le_bits(bits: &[bool]) -> u8 {
    assert!(bits.len() <= 8);

    let mut byte: u8 = 0;
    let mut base = 1;

    for bit in bits {
        if *bit {
            byte += base;
        }
        base = base.wrapping_mul(2);
    }

    byte
}
