use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::prelude::{Boolean, CondSelectGadget, EqGadget};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};

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
    pub fn construct(
        cs: ConstraintSystemRef<MNT4Fr>,
        inputs: &Vec<Vec<Boolean<MNT4Fr>>>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!inputs.is_empty());

        // Checking that the number of leaves is a power of two.
        assert_eq!((inputs.len() & (inputs.len() - 1)), 0);

        // Calculate the Pedersen hashes for the leaves.
        let mut nodes = Vec::new();

        for i in 0..inputs.len() {
            let input = &inputs[i];
            let pedersen_hash = PedersenHashGadget::evaluate(input, pedersen_generators)?;
            nodes.push(pedersen_hash);
        }

        // Calculate the rest of the tree
        let mut next_nodes = Vec::new();

        while nodes.len() > 1 {
            // Process each level of nodes.
            for j in 0..nodes.len() / 2 {
                let mut bits = Vec::new();

                // Serialize the left node.
                bits.extend(SerializeGadget::serialize_g1(cs.clone(), &nodes[2 * j])?);

                // Serialize the right node.
                bits.extend(SerializeGadget::serialize_g1(
                    cs.clone(),
                    &nodes[2 * j + 1],
                )?);

                // Calculate the parent node.
                let parent_node = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;

                next_nodes.push(parent_node);
            }
            nodes.clear();

            nodes.append(&mut next_nodes);
        }

        // Serialize the root node.
        let bits = SerializeGadget::serialize_g1(cs, &nodes[0])?;

        Ok(bits)
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
        cs: ConstraintSystemRef<MNT4Fr>,
        input: &Vec<Boolean<MNT4Fr>>,
        nodes: &Vec<G1Var>,
        path: &Vec<Boolean<MNT4Fr>>,
        root: &Vec<Boolean<MNT4Fr>>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!input.is_empty());

        // Checking that the nodes vector is not empty.
        assert!(!nodes.is_empty());

        // Checking that there is one node for each path bit.
        assert_eq!(nodes.len(), path.len());

        // Calculate the Pedersen hash for the input.
        let mut result = PedersenHashGadget::evaluate(input, pedersen_generators)?;

        // Calculate the root of the tree using the branch values.
        for i in 0..nodes.len() {
            // Decide which node is the left or the right one based on the path.
            let left_node = CondSelectGadget::conditionally_select(&path[i], &nodes[i], &result)?;

            let right_node = CondSelectGadget::conditionally_select(&path[i], &result, &nodes[i])?;

            // Serialize the left and right nodes.
            let mut bits = Vec::new();

            bits.extend(SerializeGadget::serialize_g1(cs.clone(), &left_node)?);

            bits.extend(SerializeGadget::serialize_g1(cs.clone(), &right_node)?);

            // Calculate the parent node and update result.
            result = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;
        }

        // Serialize the root node.
        let bits = SerializeGadget::serialize_g1(cs, &result)?;

        // Check that the calculated root is equal to the given root.
        root.is_eq(&bits)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::Fr as MNT4Fr;
    use ark_mnt6_753::constraints::G1Var;
    use ark_r1cs_std::prelude::{AllocVar, Boolean};
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::test_rng;
    use rand::RngCore;

    use nimiq_bls::pedersen::{pedersen_generators, pedersen_hash};
    use nimiq_bls::utils::{byte_from_le_bits, bytes_to_bits};
    use nimiq_nano_primitives::{
        merkle_tree_construct, merkle_tree_prove, merkle_tree_verify, serialize_g1_mnt6,
    };

    use super::*;

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
        for leaf in leaves {
            leaves_var.push(Vec::<Boolean<MNT4Fr>>::new_witness(cs.clone(), || Ok(leaf)).unwrap());
        }

        // Generate and allocate the Pedersen generators in the circuit.
        let generators = pedersen_generators(4);

        let mut pedersen_generators_var = vec![];
        for generator in generators {
            pedersen_generators_var.push(G1Var::new_witness(cs.clone(), || Ok(generator)).unwrap());
        }

        // Construct Merkle tree using the gadget version.
        let gadget_tree =
            MerkleTreeGadget::construct(cs, &leaves_var, &pedersen_generators_var).unwrap();

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
                bits.extend_from_slice(
                    bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref(),
                );
                bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
            } else {
                bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
                bits.extend_from_slice(
                    bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref(),
                );
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
            root,
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
                bits.extend_from_slice(
                    bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref(),
                );
                bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
            } else {
                bits.extend_from_slice(bytes_to_bits(serialize_g1_mnt6(node).as_ref()).as_ref());
                bits.extend_from_slice(
                    bytes_to_bits(serialize_g1_mnt6(other_node).as_ref()).as_ref(),
                );
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
            root,
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
