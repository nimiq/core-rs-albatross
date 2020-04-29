use algebra::mnt4_753::Fr as MNT4Fr;
use r1cs_core::SynthesisError;
use r1cs_std::bits::{boolean::Boolean, uint8::UInt8};
use r1cs_std::mnt6_753::G1Gadget;
use r1cs_std::prelude::{CondSelectGadget, EqGadget};

use crate::gadgets::mnt4::{PedersenCommitmentGadget, SerializeGadget};
use crate::utils::reverse_inner_byte_order;

/// This gadgets contains utilities to create and verify proofs for Merkle trees. It uses Pedersen
/// commitments to construct the tree instead of cryptographic hash functions in order to be more
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
    pub fn construct<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        inputs: &Vec<Vec<Boolean>>,
        pedersen_generators: &Vec<G1Gadget>,
        sum_generator: &G1Gadget,
    ) -> Result<Vec<UInt8>, SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!inputs.is_empty());

        // Checking that the number of leaves is a power of two.
        assert!(((inputs.len() & (inputs.len() - 1)) == 0));

        // Calculate the Pedersen commitments for the leaves.
        let mut nodes = Vec::new();

        for i in 0..inputs.len() {
            let input = &inputs[i];

            let pedersen_commitment = PedersenCommitmentGadget::evaluate(
                cs.ns(|| format!("pedersen commitment for leaf {}", i)),
                input,
                pedersen_generators,
                sum_generator,
            )?;

            nodes.push(pedersen_commitment);
        }

        // Calculate the rest of the tree
        let mut next_nodes = Vec::new();
        let mut i = 0;

        while nodes.len() > 1 {
            // Process each level of nodes.
            for j in 0..nodes.len() / 2 {
                let mut bits = Vec::new();

                // Serialize the left node.
                bits.extend(SerializeGadget::serialize_g1(
                    cs.ns(|| format!("serialize left node {} {}", i, j)),
                    &nodes[2 * j],
                )?);

                // Serialize the right node.
                bits.extend(SerializeGadget::serialize_g1(
                    cs.ns(|| format!("serialize right node {} {}", i, j)),
                    &nodes[2 * j + 1],
                )?);

                // Calculate the parent node.
                let parent_node = PedersenCommitmentGadget::evaluate(
                    cs.ns(|| format!("calculate parent node {} {}", i, j)),
                    &bits,
                    pedersen_generators,
                    sum_generator,
                )?;

                next_nodes.push(parent_node);
            }
            nodes.clear();
            nodes.append(&mut next_nodes);
            i += 1;
        }

        // Serialize the root node.
        let serialized_bits =
            SerializeGadget::serialize_g1(cs.ns(|| "serialize root node"), &nodes[0])?;
        let serialized_bits = reverse_inner_byte_order(&serialized_bits[..]);

        // Convert to bytes.
        let mut bytes = Vec::new();
        for i in 0..serialized_bits.len() / 8 {
            bytes.push(UInt8::from_bits_le(&serialized_bits[i * 8..(i + 1) * 8]));
        }

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
    pub fn verify<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        input: &Vec<Boolean>,
        nodes: &Vec<G1Gadget>,
        path: &Vec<Boolean>,
        root: &Vec<UInt8>,
        pedersen_generators: &Vec<G1Gadget>,
        sum_generator: &G1Gadget,
    ) -> Result<(), SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!input.is_empty());

        // Checking that the nodes vector is not empty.
        assert!(!nodes.is_empty());

        // Checking that there is one node for each path bit.
        assert_eq!(nodes.len(), path.len());

        // Calculate the Pedersen commitment for the input.
        let mut result = PedersenCommitmentGadget::evaluate(
            cs.ns(|| "pedersen commitment for input"),
            input,
            pedersen_generators,
            sum_generator,
        )?;

        // Calculate the root of the tree using the branch values.
        for i in 0..nodes.len() {
            // Decide which node is the left or the right one based on the path.
            let left_node = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally select left node {}", i)),
                &path[i],
                &nodes[i],
                &result,
            )?;

            let right_node = CondSelectGadget::conditionally_select(
                cs.ns(|| format!("conditionally select right node {}", i)),
                &path[i],
                &result,
                &nodes[i],
            )?;

            // Serialize the left and right nodes.
            let mut bits = Vec::new();

            bits.extend(SerializeGadget::serialize_g1(
                cs.ns(|| format!("serialize left node {}", i)),
                &left_node,
            )?);

            bits.extend(SerializeGadget::serialize_g1(
                cs.ns(|| format!("serialize right node {}", i)),
                &right_node,
            )?);

            // Calculate the parent node and update result.
            result = PedersenCommitmentGadget::evaluate(
                cs.ns(|| format!("calculate parent node {}", i)),
                &bits,
                pedersen_generators,
                sum_generator,
            )?;
        }

        // Serialize the root node.
        let serialized_bits =
            SerializeGadget::serialize_g1(cs.ns(|| "serialize root node"), &result)?;
        let serialized_bits = reverse_inner_byte_order(&serialized_bits[..]);

        // Convert to bytes.
        let mut bytes = Vec::new();
        for i in 0..serialized_bits.len() / 8 {
            bytes.push(UInt8::from_bits_le(&serialized_bits[i * 8..(i + 1) * 8]));
        }

        // Check that the calculated root is equal to the given root.
        root.enforce_equal(cs.ns(|| "checking equality for root"), &bytes)?;

        Ok(())
    }
}
