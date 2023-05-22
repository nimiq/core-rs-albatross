use nimiq_hash::{Blake2sHash, Hash};
#[cfg(feature = "parallel")]
use rayon::{
    iter::{IntoParallelRefIterator, ParallelIterator},
    prelude::IntoParallelIterator,
};

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
pub fn merkle_tree_construct(inputs: Vec<Vec<u8>>) -> [u8; 32] {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert!(inputs.len().is_power_of_two());

    // Calculate the Pedersen hashes for the leaves.
    #[cfg(not(feature = "parallel"))]
    let iter = inputs.into_iter();
    #[cfg(feature = "parallel")]
    let iter = inputs.into_par_iter();

    let mut nodes: Vec<Vec<u8>> = iter
        .map(|bytes| bytes.hash::<Blake2sHash>().0.to_vec())
        .collect();

    // Process each level of nodes.
    while nodes.len() > 1 {
        #[cfg(not(feature = "parallel"))]
        let iter = nodes.iter();
        #[cfg(feature = "parallel")]
        let iter = nodes.par_iter();

        // Serialize all the child nodes.
        let bytes: Vec<u8> = iter.cloned().flatten().collect();

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

        let mut next_nodes: Vec<Vec<u8>> = iter
            .map(|bytes| bytes.hash::<Blake2sHash>().0.to_vec())
            .collect();

        // Clear the child nodes and add the parent nodes.
        nodes.clear();
        nodes.append(&mut next_nodes);
    }

    let mut result = [0; 32];
    result.copy_from_slice(&nodes[0]);

    result
}
