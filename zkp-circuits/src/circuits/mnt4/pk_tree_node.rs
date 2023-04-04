use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt4_753::{constraints::PairingVar, Fq as MNT4Fq, G1Affine, G2Affine, MNT4_753};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use rand::Rng;

use nimiq_primitives::policy::Policy;

use crate::gadgets::{bits::BitVec, recursive_input::RecursiveInputVar};

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// Its purpose it two-fold:
///     1) Split the signer bitmap chunk it receives as an input into two halves.
///     2) Verify the proofs from its two child nodes in the PKTree.
/// It is different from the other node subcircuit on the MNT4 curve in that it doesn't recalculate
/// the aggregate public key commitments, it just passes them on to the next level.
#[derive(Clone)]
pub struct PKTreeNodeCircuit {
    // Constants for the circuit. Careful, changing these values result in a different circuit, so
    // whenever you change these values you need to generate new proving and verifying keys.
    tree_level: usize,
    vk_child: VerifyingKey<MNT4_753>,

    // Witnesses (private)
    l_proof: Proof<MNT4_753>,
    r_proof: Proof<MNT4_753>,

    // Inputs (public)
    l_pk_node_hash: [u8; 95],
    r_pk_node_hash: [u8; 95],
    l_agg_pk_commitment: [u8; 95],
    r_agg_pk_commitment: [u8; 95],
    signer_bitmap_chunk: Vec<bool>,
}

impl PKTreeNodeCircuit {
    pub fn new(
        tree_level: usize,
        vk_child: VerifyingKey<MNT4_753>,
        l_proof: Proof<MNT4_753>,
        r_proof: Proof<MNT4_753>,
        l_pk_node_hash: [u8; 95],
        r_pk_node_hash: [u8; 95],
        l_agg_pk_commitment: [u8; 95],
        r_agg_pk_commitment: [u8; 95],
        signer_bitmap_chunk: Vec<bool>,
    ) -> Self {
        Self {
            tree_level,
            vk_child,
            l_proof,
            r_proof,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_pk_commitment,
            r_agg_pk_commitment,
            signer_bitmap_chunk,
        }
    }

    pub fn rand<R: Rng + ?Sized>(
        tree_level: usize,
        vk_child: VerifyingKey<MNT4_753>,
        rng: &mut R,
    ) -> Self {
        let l_proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let r_proof = Proof {
            a: G1Affine::rand(rng),
            b: G2Affine::rand(rng),
            c: G1Affine::rand(rng),
        };

        let mut pk_node_hash = [0u8; 95];
        rng.fill_bytes(&mut pk_node_hash);

        let mut l_pk_node_hash = [0u8; 95];
        rng.fill_bytes(&mut l_pk_node_hash);
        let mut r_pk_node_hash = [0u8; 95];
        rng.fill_bytes(&mut r_pk_node_hash);

        let mut l_agg_pk_commitment = [0u8; 95];
        rng.fill_bytes(&mut l_agg_pk_commitment);

        let mut r_agg_pk_commitment = [0u8; 95];
        rng.fill_bytes(&mut r_agg_pk_commitment);

        let mut signer_bitmap_chunk =
            Vec::with_capacity(Policy::SLOTS as usize / 2_usize.pow(tree_level as u32));
        for _ in 0..Policy::SLOTS as usize / 2_usize.pow(tree_level as u32) {
            signer_bitmap_chunk.push(rng.gen());
        }

        PKTreeNodeCircuit {
            tree_level,
            vk_child,
            l_proof,
            r_proof,
            l_pk_node_hash,
            r_pk_node_hash,
            l_agg_pk_commitment,
            r_agg_pk_commitment,
            signer_bitmap_chunk,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fq> for PKTreeNodeCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let vk_child_var =
            VerifyingKeyVar::<MNT4_753, PairingVar>::new_constant(cs.clone(), &self.vk_child)?;

        // Allocate all the witnesses.
        let l_proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.l_proof))?;

        let r_proof_var =
            ProofVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.r_proof))?;

        // Allocate all the inputs.
        let l_pk_node_hash_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.l_pk_node_hash)?;

        let r_pk_node_hash_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.r_pk_node_hash)?;

        let l_agg_pk_commitment_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.l_agg_pk_commitment)?;

        let r_agg_pk_commitment_var =
            UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &self.r_agg_pk_commitment)?;

        let signer_bitmap_chunk_bits =
            BitVec::<MNT4Fq>::new_input_vec(cs, &self.signer_bitmap_chunk[..])?;
        let signer_bitmap_bits = signer_bitmap_chunk_bits.0
            [..Policy::SLOTS as usize / 2_usize.pow(self.tree_level as u32)]
            .to_vec();

        // Split the signer's bitmap chunk into two, for the left and right child nodes.
        let (l_signer_bitmap_bits, r_signer_bitmap_bits) = signer_bitmap_bits
            .split_at(Policy::SLOTS as usize / 2_usize.pow((self.tree_level + 1) as u32));

        // Verify the ZK proof for the left child node.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&l_pk_node_hash_var)?;
        proof_inputs.push(&l_agg_pk_commitment_var)?;
        proof_inputs.push(l_signer_bitmap_bits)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_child_var,
            &proof_inputs.into(),
            &l_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Verify the ZK proof for the right child node.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&r_pk_node_hash_var)?;
        proof_inputs.push(&r_agg_pk_commitment_var)?;
        proof_inputs.push(r_signer_bitmap_bits)?;

        Groth16VerifierGadget::<MNT4_753, PairingVar>::verify(
            &vk_child_var,
            &proof_inputs.into(),
            &r_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
