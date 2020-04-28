use std::cmp;

use algebra::mnt6_753::G1Projective;

use crate::constants::sum_generator_g1_mnt6;
use crate::primitives::{pedersen_commitment, pedersen_generators};
use crate::utils::{bytes_to_bits, serialize_g1_mnt6};

///
pub fn merkle_tree_construct(inputs: Vec<Vec<bool>>) -> Vec<u8> {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert!(!((inputs.len() & (inputs.len() - 1)) == 0));

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

pub fn merkle_tree_verify(
    input: Vec<bool>,
    nodes: Vec<G1Projective>,
    path: Vec<bool>,
    root: Vec<u8>,
) -> () {
    // Checking that the inputs vector is not empty.
    assert!(!input.is_empty());

    // Checking that the nodes vector is not empty.
    assert!(!nodes.is_empty());

    // Checking that the inputs vector is not empty.
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

    // Check that the calculated root is equal to the given root.
    assert_eq!(root, reference);
}
