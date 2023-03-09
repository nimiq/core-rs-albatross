use ark_mnt6_753::G2Projective;
#[cfg(feature = "parallel")]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use nimiq_primitives::policy::Policy;

use crate::merkle_tree::merkle_tree_construct;
use crate::serialize::serialize_g2_mnt6;

/// This is the depth of the PKTree circuit.
pub const PK_TREE_DEPTH: usize = 5;

/// This is the number of leaves in the PKTree circuit.
pub const PK_TREE_BREADTH: usize = 2_usize.pow(PK_TREE_DEPTH as u32);

/// This function is meant to calculate the public key tree "off-circuit". Generating the public key
/// tree with this function guarantees that it is compatible with the ZK circuit.
// These should be removed once pk_tree_root does something different than returning default
pub fn pk_tree_construct(public_keys: Vec<G2Projective>) -> [u8; 95] {
    // Checking that the number of public keys is equal to the number of validator slots.
    assert_eq!(public_keys.len(), Policy::SLOTS as usize);

    // Checking that the number of public keys is a multiple of the number of leaves.
    assert_eq!(public_keys.len() % PK_TREE_BREADTH, 0);

    // Serialize the public keys into bits.
    #[cfg(not(feature = "parallel"))]
    let iter = public_keys.iter();
    #[cfg(feature = "parallel")]
    let iter = public_keys.par_iter();
    let bytes: Vec<u8> = iter
        .map(|pk| serialize_g2_mnt6(pk).to_vec())
        .flatten()
        .collect();

    // Chunk the bits into the number of leaves.
    let mut inputs = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        inputs.push(
            bytes[i * bytes.len() / PK_TREE_BREADTH..(i + 1) * bytes.len() / PK_TREE_BREADTH]
                .to_vec(),
        );
    }

    // Calculate the merkle tree root.
    merkle_tree_construct(inputs)
}
