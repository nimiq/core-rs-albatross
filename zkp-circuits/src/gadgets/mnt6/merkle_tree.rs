use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq};
use ark_r1cs_std::{
    prelude::{Boolean, CondSelectGadget, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::serialize::SerializeGadget;

use super::{DefaultPedersenHashGadget, DefaultPedersenParametersVar};

/// This gadgets contains utilities to create Merkle trees and verify proofs for them. It uses Pedersen
/// hashes to construct the tree instead of cryptographic hash functions in order to be more efficient.
pub struct MerkleTreeGadget;

impl MerkleTreeGadget {
    /// Creates a Merkle tree from the given inputs, as a vector of vectors of Booleans, and outputs
    /// the root. Each vector of Booleans is meant to be one leaf. Each leaf can be of a different
    /// size. Number of leaves has to be a power of two.
    /// The tree is constructed from left to right. For example, if we are given inputs {0, 1, 2, 3}
    /// then the resulting tree will be:
    ///                      o
    ///                    /   \
    ///                   o     o
    ///                  / \   / \
    ///                 0  1  2  3
    #[allow(dead_code)]
    pub fn construct(
        cs: ConstraintSystemRef<MNT6Fq>,
        inputs: &[Vec<UInt8<MNT6Fq>>],
        pedersen_generators: &DefaultPedersenParametersVar,
    ) -> Result<Vec<UInt8<MNT6Fq>>, SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!inputs.is_empty());

        // Checking that the number of leaves is a power of two.
        assert_eq!((inputs.len() & (inputs.len() - 1)), 0);

        // Calculate the Pedersen hashes for the leaves.
        let mut nodes = Vec::new();

        for input in inputs {
            let pedersen_hash = DefaultPedersenHashGadget::evaluate(input, pedersen_generators)?;
            nodes.push(pedersen_hash);
        }

        // Calculate the rest of the tree
        let mut next_nodes = Vec::new();

        while nodes.len() > 1 {
            // Process each level of nodes.
            for j in 0..nodes.len() / 2 {
                let mut bytes = Vec::new();

                // Serialize the left node.
                bytes.extend(nodes[2 * j].serialize_compressed(cs.clone())?);

                // Serialize the right node.
                bytes.extend(nodes[2 * j + 1].serialize_compressed(cs.clone())?);

                // Calculate the parent node.
                let parent_node = DefaultPedersenHashGadget::evaluate(&bytes, pedersen_generators)?;

                next_nodes.push(parent_node);
            }
            nodes.clear();

            nodes.append(&mut next_nodes);
        }

        // Serialize the root node.
        let bytes = nodes[0].serialize_compressed(cs)?;

        Ok(bytes)
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
    pub fn verify(
        cs: ConstraintSystemRef<MNT6Fq>,
        input: &[UInt8<MNT6Fq>],
        nodes: &[G1Var],
        path: &[Boolean<MNT6Fq>],
        root: &[UInt8<MNT6Fq>],
        pedersen_generators: &DefaultPedersenParametersVar,
    ) -> Result<Boolean<MNT6Fq>, SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!input.is_empty());

        // Checking that the nodes vector is not empty.
        assert!(!nodes.is_empty());

        // Checking that there is one node for each path bit.
        assert_eq!(nodes.len(), path.len());

        // Calculate the Pedersen hash for the input.
        let mut result = DefaultPedersenHashGadget::evaluate(input, pedersen_generators)?;

        // Calculate the root of the tree using the branch values.
        for i in 0..nodes.len() {
            // Decide which node is the left or the right one based on the path.
            let left_node = CondSelectGadget::conditionally_select(&path[i], &nodes[i], &result)?;

            let right_node = CondSelectGadget::conditionally_select(&path[i], &result, &nodes[i])?;

            // Serialize the left and right nodes.
            let mut bytes = Vec::new();

            bytes.extend(left_node.serialize_compressed(cs.clone())?);

            bytes.extend(right_node.serialize_compressed(cs.clone())?);

            // Calculate the parent node and update result.
            result = DefaultPedersenHashGadget::evaluate(&bytes, pedersen_generators)?;
        }

        // Serialize the root node.
        let bytes = result.serialize_compressed(cs)?;

        // Check that the calculated root is equal to the given root.
        root.is_eq(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq};
    use ark_r1cs_std::{
        prelude::{AllocVar, Boolean},
        R1CSVar,
    };
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::test_rng;

    use rand::{Rng, RngCore};

    use nimiq_test_log::test;

    use nimiq_zkp_primitives::{
        merkle_tree_construct, merkle_tree_prove, merkle_tree_verify,
        pedersen::default_pedersen_hash, serialize_g1_mnt6, PEDERSEN_PARAMETERS,
    };

    use crate::{gadgets::pedersen::PedersenParametersVar, utils::byte_to_le_bits};

    use super::*;

    #[test]
    fn merkle_tree_construct_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random bits.
        let mut bytes = [0u8; 128];
        let mut leaves = Vec::new();
        for _ in 0..16 {
            rng.fill_bytes(&mut bytes);
            leaves.push(bytes.to_vec());
        }

        // Construct Merkle tree using the primitive version.
        let primitive_tree = &merkle_tree_construct(leaves.clone());

        // Allocate the random bits in the circuit.
        let mut leaves_var = vec![];
        for leaf in leaves {
            leaves_var.push(Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(leaf)).unwrap());
        }

        // Allocate the generators.
        let generators_var =
            PedersenParametersVar::new_witness(cs.clone(), || Ok(&*PEDERSEN_PARAMETERS)).unwrap();

        // Construct Merkle tree using the gadget version.
        let gadget_tree = MerkleTreeGadget::construct(cs, &leaves_var, &generators_var).unwrap();

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
            leaves.push(bytes.to_vec());
        }

        // Create random position.
        let mut path: u8 = rng.gen();
        path &= 0x0f;
        let position = path as usize;
        let mut path = byte_to_le_bits(path);
        path.truncate(4);

        // Calculate root.
        let root = merkle_tree_construct(leaves.clone());

        // Calculate proof.
        let proof = merkle_tree_prove(leaves.clone(), path.clone());

        // Verify proof.
        let input = leaves.get(position).unwrap();

        assert!(merkle_tree_verify(input, proof, path, root))
    }

    #[test]
    fn merkle_tree_verify_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random bits.
        let mut leaf = [0u8; 128];
        rng.fill_bytes(&mut leaf);

        // Create fake Merkle tree branch.
        let path = vec![false, true, false, true];
        let mut hash_bytes = [0u8; 95];
        let mut nodes = vec![];
        let mut bytes = vec![];

        let mut node = default_pedersen_hash(&leaf);

        for i in 0..4 {
            rng.fill_bytes(&mut hash_bytes);
            let other_node = default_pedersen_hash(&hash_bytes);

            if path[i] {
                bytes.extend_from_slice(serialize_g1_mnt6(&other_node).as_ref());
                bytes.extend_from_slice(serialize_g1_mnt6(&node).as_ref());
            } else {
                bytes.extend_from_slice(serialize_g1_mnt6(&node).as_ref());
                bytes.extend_from_slice(serialize_g1_mnt6(&other_node).as_ref());
            }

            nodes.push(other_node);
            node = default_pedersen_hash(&bytes);
            bytes.clear();
        }

        // Create root.
        let root = serialize_g1_mnt6(&node);

        // Verify Merkle proof using the primitive version.
        assert!(merkle_tree_verify(
            &leaf,
            nodes.clone(),
            path.clone(),
            root.clone(),
        ));

        // Allocate the leaf in the circuit.
        let leaf_var = Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(leaf)).unwrap();

        // Allocate the nodes in the circuit.
        let nodes_var = Vec::<G1Var>::new_witness(cs.clone(), || Ok(nodes)).unwrap();

        // Allocate the path in the circuit.
        let path_var = Vec::<Boolean<MNT6Fq>>::new_witness(cs.clone(), || Ok(path)).unwrap();

        // Allocate the root in the circuit.
        let root_var = Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(root)).unwrap();

        // Allocate the Pedersen generators in the circuit.
        let generators_var =
            PedersenParametersVar::new_witness(cs.clone(), || Ok(&*PEDERSEN_PARAMETERS)).unwrap();

        // Verify Merkle proof.
        assert!(MerkleTreeGadget::verify(
            cs,
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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random bits.
        let mut leaf = [0u8; 128];
        rng.fill_bytes(&mut leaf);

        // Create fake Merkle tree branch.
        let path = vec![false, true, false, true];
        let mut bytes = [0u8; 95];
        let mut nodes = vec![];
        let mut out_bytes = vec![];

        let mut node = default_pedersen_hash(&leaf);

        for i in 0..4 {
            rng.fill_bytes(&mut bytes);
            let other_node = default_pedersen_hash(&bytes);

            if path[i] {
                out_bytes.extend_from_slice(serialize_g1_mnt6(&other_node).as_ref());
                out_bytes.extend_from_slice(serialize_g1_mnt6(&node).as_ref());
            } else {
                out_bytes.extend_from_slice(serialize_g1_mnt6(&node).as_ref());
                out_bytes.extend_from_slice(serialize_g1_mnt6(&other_node).as_ref());
            }

            nodes.push(other_node);
            node = default_pedersen_hash(&out_bytes);
            out_bytes.clear();
        }

        // Create wrong root.
        let mut root = [0u8; 95];
        rng.fill_bytes(&mut root);

        // Verify Merkle proof using the primitive version.
        assert!(!merkle_tree_verify(
            &leaf,
            nodes.clone(),
            path.clone(),
            root.clone(),
        ));

        // Allocate the leaf in the circuit.
        let leaf_var = Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(leaf)).unwrap();

        // Allocate the nodes in the circuit.
        let nodes_var = Vec::<G1Var>::new_witness(cs.clone(), || Ok(nodes)).unwrap();

        // Allocate the path in the circuit.
        let path_var = Vec::<Boolean<MNT6Fq>>::new_witness(cs.clone(), || Ok(path)).unwrap();

        // Allocate the root in the circuit.
        let root_var = Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(root)).unwrap();

        // Allocate the Pedersen generators in the circuit.
        let generators_var =
            PedersenParametersVar::new_witness(cs.clone(), || Ok(&*PEDERSEN_PARAMETERS)).unwrap();

        // Verify Merkle proof.
        assert!(!MerkleTreeGadget::verify(
            cs,
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
}
