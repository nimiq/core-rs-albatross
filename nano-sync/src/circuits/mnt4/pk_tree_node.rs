

use ark_crypto_primitives::snark::BooleanInputVar;
use ark_crypto_primitives::SNARKGadget;
use ark_groth16::constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar};
use ark_groth16::{Proof, VerifyingKey};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{FqVar, G1Var, G2Var, PairingVar};
use ark_mnt6_753::{Fq, G2Projective, MNT6_753};
use ark_r1cs_std::prelude::{AllocVar, Boolean, CurveVar, EqGadget};
use ark_r1cs_std::ToBitsGadget;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};


use crate::constants::{PK_TREE_DEPTH, VALIDATOR_SLOTS};
use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};
use crate::primitives::pedersen_generators;
use crate::utils::{pack_inputs, unpack_inputs};
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// It is different from the other node subcircuit on the MNT6 curve in that it does recalculate
/// the aggregate public key commitments.
#[derive(Clone)]
pub struct PKTreeNodeCircuit {
    // Verifying key of the circuit of the child node. Not an input to the SNARK circuit.
    vk_child: VerifyingKey<MNT6_753>,

    // Witnesses (private)
    left_proof: Proof<MNT6_753>,
    right_proof: Proof<MNT6_753>,
    agg_pk_chunks: Vec<G2Projective>,

    // Inputs (public)
    // Our inputs are always vectors of booleans (semantically), so that they are consistent across
    // the different elliptic curves that we use. However, for compactness, we represent them as
    // field elements. Both of the curves that we use have a modulus of 753 bits and a capacity
    // of 752 bits. So, the first 752 bits (in little-endian) of each field element is data, and the
    // last bit is always set to zero.
    pk_tree_root: Vec<Fq>,
    agg_pk_commitment: Vec<Fq>,
    signer_bitmap: Fq,
    path: Fq,
}

impl PKTreeNodeCircuit {
    pub fn new(
        vk_child: VerifyingKey<MNT6_753>,
        left_proof: Proof<MNT6_753>,
        right_proof: Proof<MNT6_753>,
        agg_pk_chunks: Vec<G2Projective>,
        pk_tree_root: Vec<Fq>,
        agg_pk_commitment: Vec<Fq>,
        signer_bitmap: Fq,
        path: Fq,
    ) -> Self {
        Self {
            vk_child,
            left_proof,
            right_proof,
            agg_pk_chunks,
            pk_tree_root,
            agg_pk_commitment,
            signer_bitmap,
            path,
        }
    }
}

impl ConstraintSynthesizer<MNT4Fr> for PKTreeNodeCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT4Fr>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");

        let pedersen_generators_var =
            Vec::<G1Var>::new_constant(cs.clone(), pedersen_generators(5))?;

        let vk_child_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), &self.vk_child)?;

        // Allocate all the witnesses.
        next_cost_analysis!(cs, cost, || { "Alloc witnesses" });

        let left_proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.left_proof))?;

        let right_proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.right_proof))?;

        let agg_pk_chunks_var =
            Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.agg_pk_chunks[..]))?;

        // Allocate all the inputs.
        next_cost_analysis!(cs, cost, || { "Alloc inputs" });

        let pk_tree_root_var = Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.pk_tree_root[..]))?;

        let agg_pk_commitment_var =
            Vec::<FqVar>::new_input(cs.clone(), || Ok(&self.agg_pk_commitment[..]))?;

        let signer_bitmap_var = FqVar::new_input(cs.clone(), || Ok(&self.signer_bitmap))?;

        let path_var = FqVar::new_input(cs.clone(), || Ok(&self.path))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        next_cost_analysis!(cs, cost, || { "Unpack inputs" });

        let pk_tree_root_bits = unpack_inputs(pk_tree_root_var)?[..760].to_vec();

        let agg_pk_commitment_bits = unpack_inputs(agg_pk_commitment_var)?[..760].to_vec();

        let signer_bitmap_bits =
            unpack_inputs(vec![signer_bitmap_var])?[..VALIDATOR_SLOTS].to_vec();

        let mut path_bits = unpack_inputs(vec![path_var])?[..PK_TREE_DEPTH].to_vec();

        // Calculating the aggregate public key.
        next_cost_analysis!(cs, cost, || { "Calculate agg pk" });

        let mut agg_pk = G2Var::zero();

        for pk in &agg_pk_chunks_var {
            agg_pk += pk;
        }

        // Verifying aggregate public key commitment. It just checks that the calculated aggregate
        // public key is correct by comparing it with the aggregate public key commitment given as
        // an input.
        next_cost_analysis!(cs, cost, || { "Verify agg pk" });

        let agg_pk_bits = SerializeGadget::serialize_g2(cs.clone(), &agg_pk)?;

        let pedersen_hash = PedersenHashGadget::evaluate(&agg_pk_bits, &pedersen_generators_var)?;

        let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

        agg_pk_commitment_bits.enforce_equal(&pedersen_bits)?;

        // Calculating the commitments to each of the aggregate public keys chunks. These
        // will be given as input to the SNARK circuits lower on the tree.
        next_cost_analysis!(cs, cost, || { "Calculate agg pk chunks commitments" });

        let mut agg_pk_chunks_commitments = Vec::new();

        for chunk in &agg_pk_chunks_var {
            let chunk_bits = SerializeGadget::serialize_g2(cs.clone(), chunk)?;

            let pedersen_hash =
                PedersenHashGadget::evaluate(&chunk_bits, &pedersen_generators_var)?;

            let pedersen_bits = SerializeGadget::serialize_g1(cs.clone(), &pedersen_hash)?;

            agg_pk_chunks_commitments.push(pedersen_bits);
        }

        // Calculate the path for the left and right child nodes. Given the current position P,
        // the left position L and the right position R are given as:
        //    L = 2 * P
        //    R = 2 * P + 1
        // For efficiency reasons, we actually calculate the path using bit manipulation.
        next_cost_analysis!(cs, cost, || { "Calculate paths" });

        // Calculate P >> 1, which is equivalent to calculating 2 * P (in little-endian).
        path_bits.pop();
        path_bits.insert(0, Boolean::Constant(false));
        let left_path = path_bits.clone();

        // path_bits is currently P >> 1 = L. Calculate L & 1, which is equivalent to L + 1 (in little-endian).
        let _ = path_bits.remove(0);
        path_bits.insert(0, Boolean::Constant(true));
        let right_path = path_bits;

        // Verify the ZK proof for the left child node.
        next_cost_analysis!(cs, cost, || { "Verify left ZK proof" });

        let mut proof_inputs = pack_inputs(pk_tree_root_bits.clone());

        proof_inputs.append(&mut pack_inputs(agg_pk_chunks_commitments[0].to_bits_le()?));

        proof_inputs.append(&mut pack_inputs(agg_pk_chunks_commitments[1].to_bits_le()?));

        proof_inputs.append(&mut pack_inputs(signer_bitmap_bits.clone()));

        proof_inputs.append(&mut pack_inputs(left_path));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_child_var,
            &input_var,
            &left_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Verify the ZK proof for the right child node.
        next_cost_analysis!(cs, cost, || { "Verify right ZK proof" });

        let mut proof_inputs = pack_inputs(pk_tree_root_bits);

        proof_inputs.append(&mut pack_inputs(agg_pk_chunks_commitments[2].to_bits_le()?));

        proof_inputs.append(&mut pack_inputs(agg_pk_chunks_commitments[3].to_bits_le()?));

        proof_inputs.append(&mut pack_inputs(signer_bitmap_bits));

        proof_inputs.append(&mut pack_inputs(right_path));

        let input_var = BooleanInputVar::new(proof_inputs);

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_child_var,
            &input_var,
            &right_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}
