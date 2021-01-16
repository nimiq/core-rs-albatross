use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::prelude::{Boolean, CondSelectGadget, EqGadget};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};

/// This gadgets contains utilities to create and verify proofs for Merkle trees. It uses Pedersen
/// hashes to construct the tree instead of cryptographic hash functions in order to be more
/// efficient.
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
