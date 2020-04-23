use algebra::mnt4_753::Fr as MNT4Fr;
use r1cs_core::SynthesisError;
use r1cs_std::bits::{boolean::Boolean, uint8::UInt8};
use r1cs_std::mnt6_753::G1Gadget;

use crate::gadgets::mnt4::{PedersenCommitmentGadget, SerializeGadget};
use crate::utils::reverse_inner_byte_order;

///
pub struct MerkleTreeGadget;

impl MerkleTreeGadget {
    /// Creates a Merkle tree from the given inputs, as a vector of vectors of Booleans, and outputs
    /// the root. Each vector of Booleans is meant to be one leaf. Each leaf can be of a different
    /// size.
    pub fn construct<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        inputs: &Vec<Vec<Boolean>>,
        pedersen_generators: &Vec<G1Gadget>,
        sum_generator: &G1Gadget,
    ) -> Result<Vec<UInt8>, SynthesisError> {
        // Checking that the inputs vector is not empty.
        assert!(!inputs.is_empty());

        // Checking that the number of leaves is a power of two.
        assert!(!((inputs.len() & (inputs.len() - 1)) == 0));

        // Checking that we have enough generators to calculate the Pedersen commitments for the nodes.
        assert!(pedersen_generators.len() > 2);

        // Calculate the Pedersen commitments for the leaves.
        let mut nodes = Vec::new();

        for i in 0..inputs.len() {
            let input = &inputs[i];

            assert!(pedersen_generators.len() > input.len());

            let pedersen_commitment = PedersenCommitmentGadget::evaluate(
                cs.ns(|| format!("pedersen commitment for leaf {}", i)),
                pedersen_generators,
                input,
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
                    pedersen_generators,
                    &bits,
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
}
