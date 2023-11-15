use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt6_753::{
    constraints::{G2Var, PairingVar},
    Fq as MNT6Fq, G1Affine, G2Affine, G2Projective, MNT6_753,
};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_pedersen_generators::GenericWindow;
use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::pedersen_parameters_mnt6;
use rand::Rng;

use crate::{
    blake2s::evaluate_blake2s,
    gadgets::{
        bits::BitVec, mnt6::DefaultPedersenParametersVar, pedersen::PedersenHashGadget,
        recursive_input::RecursiveInputVar, serialize::SerializeGadget,
    },
};

pub(super) type PkInnerNodeWindow = GenericWindow<4, MNT6Fq>;

pub(super) fn hash_g2(
    cs: &ConstraintSystemRef<MNT6Fq>,
    g2: &G2Var,
    pedersen_generators_var: &DefaultPedersenParametersVar,
) -> Result<Vec<UInt8<MNT6Fq>>, SynthesisError> {
    let bytes = g2.serialize_compressed(cs.clone())?;

    let pedersen_hash =
        PedersenHashGadget::<_, _, PkInnerNodeWindow>::evaluate(&bytes, pedersen_generators_var)?;

    pedersen_hash.serialize_compressed(cs.clone())
}

/// This is the node subcircuit of the PKTreeCircuit. See PKTreeLeafCircuit for more details.
/// Its purpose it three-fold:
///     1) Split the signer bitmap chunk it receives as an input into two halves.
///     2) Verify the proofs from its two child nodes in the PKTree.
///     3) Verify the aggregate public key commitments.
///     4) Verify the node hash.
/// This circuit must also calculate the public input for the lower level because the lower
/// level is MNT4 but all calculations must be performed on MNT6.
#[derive(Clone)]
pub struct PKTreeNodeCircuit {
    // Constants for the circuit. Careful, changing these values result in a different circuit, so
    // whenever you change these values you need to generate new proving and verifying keys.
    tree_level: usize,
    vk_child: VerifyingKey<MNT6_753>,

    // Witnesses (private)
    l_proof: Proof<MNT6_753>,
    r_proof: Proof<MNT6_753>,
    ll_agg_pk_commitment: G2Projective,
    lr_agg_pk_commitment: G2Projective,
    rl_agg_pk_commitment: G2Projective,
    rr_agg_pk_commitment: G2Projective,

    ll_pk_node_hash: [u8; 32],
    lr_pk_node_hash: [u8; 32],
    rl_pk_node_hash: [u8; 32],
    rr_pk_node_hash: [u8; 32],

    // Inputs (public)
    pk_node_hash: [u8; 32],
    agg_pk_commitment: [u8; 95],
    signer_bitmap_chunk: Vec<bool>,
}

impl PKTreeNodeCircuit {
    pub fn new(
        tree_level: usize,
        vk_child: VerifyingKey<MNT6_753>,
        l_proof: Proof<MNT6_753>,
        r_proof: Proof<MNT6_753>,
        ll_agg_pk_commitment: G2Projective,
        lr_agg_pk_commitment: G2Projective,
        rl_agg_pk_commitment: G2Projective,
        rr_agg_pk_commitment: G2Projective,
        ll_pk_node_hash: [u8; 32],
        lr_pk_node_hash: [u8; 32],
        rl_pk_node_hash: [u8; 32],
        rr_pk_node_hash: [u8; 32],
        pk_node_hash: [u8; 32],
        agg_pk_commitment: [u8; 95],
        signer_bitmap_chunk: Vec<bool>,
    ) -> Self {
        Self {
            tree_level,
            vk_child,
            l_proof,
            r_proof,
            ll_agg_pk_commitment,
            lr_agg_pk_commitment,
            rl_agg_pk_commitment,
            rr_agg_pk_commitment,
            ll_pk_node_hash,
            lr_pk_node_hash,
            rl_pk_node_hash,
            rr_pk_node_hash,
            pk_node_hash,
            agg_pk_commitment,
            signer_bitmap_chunk,
        }
    }

    pub fn rand<R: Rng + ?Sized>(
        tree_level: usize,
        vk_child: VerifyingKey<MNT6_753>,
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

        let agg_pks = vec![G2Projective::rand(rng); 4];

        let mut pk_node_hashes = vec![];
        for _ in 0..4 {
            let mut pk_node_hash = [0u8; 32];
            rng.fill_bytes(&mut pk_node_hash);
            pk_node_hashes.push(pk_node_hash);
        }
        let mut pk_node_hash = [0u8; 32];
        rng.fill_bytes(&mut pk_node_hash);

        let mut agg_pk_commitment = [0u8; 95];
        rng.fill_bytes(&mut agg_pk_commitment);

        let mut signer_bitmap =
            Vec::with_capacity(Policy::SLOTS as usize / 2_usize.pow(tree_level as u32));
        for _ in 0..Policy::SLOTS as usize / 2_usize.pow(tree_level as u32) {
            signer_bitmap.push(rng.gen());
        }

        PKTreeNodeCircuit::new(
            tree_level,
            vk_child,
            l_proof,
            r_proof,
            agg_pks[0],
            agg_pks[1],
            agg_pks[2],
            agg_pks[3],
            pk_node_hashes[0],
            pk_node_hashes[1],
            pk_node_hashes[2],
            pk_node_hashes[3],
            pk_node_hash,
            agg_pk_commitment,
            signer_bitmap,
        )
    }
}

impl ConstraintSynthesizer<MNT6Fq> for PKTreeNodeCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let pedersen_generators_var = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<PkInnerNodeWindow>(),
        )?;

        let vk_child_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_constant(cs.clone(), &self.vk_child)?;

        // Allocate all the witnesses.
        let l_proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.l_proof))?;

        let r_proof_var =
            ProofVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(&self.r_proof))?;

        let ll_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.ll_agg_pk_commitment))?;
        let lr_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.lr_agg_pk_commitment))?;
        let rl_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.rl_agg_pk_commitment))?;
        let rr_agg_pk_commitment_var =
            G2Var::new_witness(cs.clone(), || Ok(self.rr_agg_pk_commitment))?;

        let ll_pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.ll_pk_node_hash)?;
        let lr_pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.lr_pk_node_hash)?;
        let rl_pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.rl_pk_node_hash)?;
        let rr_pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &self.rr_pk_node_hash)?;

        // Allocate all the inputs.
        let pk_node_hash_bytes = UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.pk_node_hash)?;

        let agg_pk_commitment_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.agg_pk_commitment)?;

        let signer_bitmap_chunk_bytes =
            BitVec::<MNT6Fq>::new_input_vec(cs.clone(), &self.signer_bitmap_chunk)?;
        let signer_bitmap_chunk_bits = signer_bitmap_chunk_bytes.0
            [..Policy::SLOTS as usize / 2_usize.pow(self.tree_level as u32)]
            .to_vec();

        // Calculating the aggregate public key.
        let l_agg_pk_commitment_var = &ll_agg_pk_commitment_var + &lr_agg_pk_commitment_var;
        let r_agg_pk_commitment_var = &rl_agg_pk_commitment_var + &rr_agg_pk_commitment_var;
        let agg_pk_commitment_var = l_agg_pk_commitment_var + r_agg_pk_commitment_var;

        // Verifying aggregate public key commitment. It just checks that the calculated aggregate
        // public key is correct by comparing it with the aggregate public key commitment given as
        // an input.
        let pedersen_bytes = hash_g2(&cs, &agg_pk_commitment_var, &pedersen_generators_var)?;

        agg_pk_commitment_bytes.enforce_equal(&pedersen_bytes)?;

        // Calculating the commitments to each of the aggregate public keys chunks. These
        // will be given as input to the SNARK circuits lower on the tree.
        let ll_agg_pk_commitment_bytes =
            hash_g2(&cs, &ll_agg_pk_commitment_var, &pedersen_generators_var)?;
        let lr_agg_pk_commitment_bytes =
            hash_g2(&cs, &lr_agg_pk_commitment_var, &pedersen_generators_var)?;
        let rl_agg_pk_commitment_bytes =
            hash_g2(&cs, &rl_agg_pk_commitment_var, &pedersen_generators_var)?;
        let rr_agg_pk_commitment_bytes =
            hash_g2(&cs, &rr_agg_pk_commitment_var, &pedersen_generators_var)?;

        // Calculate the node hashes of our children and our own.
        let mut l_pk_node_hash_bytes = vec![];
        l_pk_node_hash_bytes.extend_from_slice(&ll_pk_node_hash_bytes);
        l_pk_node_hash_bytes.extend_from_slice(&lr_pk_node_hash_bytes);
        let l_pk_node_hash_bytes = evaluate_blake2s(&l_pk_node_hash_bytes)?;

        let mut r_pk_node_hash_bytes = vec![];
        r_pk_node_hash_bytes.extend_from_slice(&rl_pk_node_hash_bytes);
        r_pk_node_hash_bytes.extend_from_slice(&rr_pk_node_hash_bytes);
        let r_pk_node_hash_bytes = evaluate_blake2s(&r_pk_node_hash_bytes)?;

        let mut calculated_pk_node_hash_bytes = vec![];
        calculated_pk_node_hash_bytes.extend_from_slice(&l_pk_node_hash_bytes);
        calculated_pk_node_hash_bytes.extend_from_slice(&r_pk_node_hash_bytes);
        let calculated_pk_node_hash_bytes = evaluate_blake2s(&calculated_pk_node_hash_bytes)?;

        calculated_pk_node_hash_bytes.enforce_equal(&pk_node_hash_bytes)?;

        // Split the signer's bitmap chunk into two, for the left and right child nodes.
        let (l_signer_bitmap_bits, r_signer_bitmap_bits) = signer_bitmap_chunk_bits
            .split_at(Policy::SLOTS as usize / 2_usize.pow((self.tree_level + 1) as u32));

        // Verify the ZK proof for the left child node.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&ll_pk_node_hash_bytes)?;
        proof_inputs.push(&lr_pk_node_hash_bytes)?;
        proof_inputs.push(&ll_agg_pk_commitment_bytes)?;
        proof_inputs.push(&lr_agg_pk_commitment_bytes)?;
        proof_inputs.push(l_signer_bitmap_bits)?;

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_child_var,
            &proof_inputs.into(),
            &l_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        // Verify the ZK proof for the right child node.
        let mut proof_inputs = RecursiveInputVar::new();
        proof_inputs.push(&rl_pk_node_hash_bytes)?;
        proof_inputs.push(&rr_pk_node_hash_bytes)?;
        proof_inputs.push(&rl_agg_pk_commitment_bytes)?;
        proof_inputs.push(&rr_agg_pk_commitment_bytes)?;
        proof_inputs.push(r_signer_bitmap_bits)?;

        Groth16VerifierGadget::<MNT6_753, PairingVar>::verify(
            &vk_child_var,
            &proof_inputs.into(),
            &r_proof_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}
