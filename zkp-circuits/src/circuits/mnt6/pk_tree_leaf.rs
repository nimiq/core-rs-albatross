use ark_ff::UniformRand;
use ark_mnt6_753::{constraints::G2Var, Fq as MNT6Fq, G2Projective, MNT6_753};
use ark_r1cs_std::{
    prelude::{AllocVar, CondSelectGadget, CurveVar, EqGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_std::Zero;
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_primitives::{policy::Policy, slots_allocation::PK_TREE_BREADTH};
use nimiq_zkp_primitives::{
    pedersen::default_pedersen_hash, pedersen_parameters_mnt6, serialize_g1_mnt6, serialize_g2_mnt6,
};
use rand::{distributions::Standard, prelude::Distribution};

use crate::{
    blake2s::evaluate_blake2s,
    gadgets::{
        bits::BitVec,
        mnt6::{DefaultPedersenHashGadget, DefaultPedersenParametersVar},
        serialize::SerializeGadget,
    },
};

/// This is the leaf subcircuit of the PKTreeCircuit. This circuit main function is to process the
/// validator's public keys and "return" the aggregate public key for the Macro Block. At a
/// high-level, it divides all the computation into 2^n parts, where n is the depth of the tree, so
/// that each part uses only a manageable amount of memory and can be run on consumer hardware.
/// It does this by forming a binary tree of recursive SNARKs. Each of the 2^n leaves receives
/// Merkle tree commitments to the public keys list and a commitment to the corresponding aggregate
/// public key chunk (there are 2^n chunks, one for each leaf) in addition to the part of the
/// signer's bitmap relevant to the leaf position.
/// Each of the leaves then checks that its specific chunk of the public keys,
/// aggregated according to its specific chunk of the signer's bitmap, matches the corresponding
/// chunk of the aggregated public key.
/// All of the other upper levels of the recursive SNARK tree just verify SNARK proofs for its child
/// nodes and recursively aggregate the aggregate public key chunks (no pun intended).
/// At a lower-level, this circuit does two things:
///     1. That the public keys given as witness hash to the expected leaf's hash.
///     2. That the public keys given as witness, when aggregated according to the signer's bitmap
///        (given as an input), match the aggregated public key commitment (also given as an input).
#[derive(Clone)]
pub struct PKTreeLeafCircuit {
    // Witnesses (private)
    pks: Vec<G2Projective>,

    // Inputs (public)
    pk_node_hash: [u8; 32],
    agg_pk_commitment: [u8; 95],
    signer_bitmap_chunk: Vec<bool>,
}

impl PKTreeLeafCircuit {
    pub fn num_inputs(tree_level: usize) -> usize {
        let num_bits = Policy::SLOTS as usize / 2_usize.pow(tree_level as u32);
        crate::circuits::num_inputs::<MNT6_753>(&[32, 95, num_bits.div_ceil(8)])
    }

    pub fn new(
        pks: Vec<G2Projective>,
        pk_node_hash: [u8; 32],
        agg_pk_commitment: [u8; 95],
        signer_bitmap: Vec<bool>,
    ) -> Self {
        Self {
            pks,
            pk_node_hash,
            agg_pk_commitment,
            signer_bitmap_chunk: signer_bitmap,
        }
    }

    fn sample_with_len<R: rand::Rng + ?Sized>(rng: &mut R, len: usize) -> PKTreeLeafCircuit {
        let pks = vec![G2Projective::rand(rng); len];

        let mut pk_tree_root = [0u8; 32];
        rng.fill_bytes(&mut pk_tree_root);

        let mut agg_pk_commitment = [0u8; 95];
        rng.fill_bytes(&mut agg_pk_commitment);

        let mut signer_bitmap = Vec::with_capacity(len);
        for _ in 0..len {
            signer_bitmap.push(rng.gen());
        }

        // Create parameters for our circuit
        PKTreeLeafCircuit::new(pks, pk_tree_root, agg_pk_commitment, signer_bitmap)
    }

    pub fn random_with_len<R: rand::Rng + ?Sized>(rng: &mut R, len: usize) -> PKTreeLeafCircuit {
        let pks = vec![G2Projective::rand(rng); len];

        let mut signer_bitmap = Vec::with_capacity(len);
        for _ in 0..len {
            signer_bitmap.push(rng.gen());
        }

        let mut agg_pk = G2Projective::zero();
        let mut pk_node_hash = vec![];

        for (i, pk) in pks.iter().enumerate() {
            pk_node_hash.extend(serialize_g2_mnt6(pk));
            if signer_bitmap[i] {
                agg_pk += pk;
            }
        }
        let pk_node_hash = pk_node_hash.hash::<Blake2sHash>().0;

        let agg_pk_bytes = serialize_g2_mnt6(&agg_pk);

        let hash = default_pedersen_hash::<MNT6_753>(&agg_pk_bytes);
        let agg_pk_commitment = serialize_g1_mnt6(&hash);

        // Create parameters for our circuit
        PKTreeLeafCircuit::new(
            pks.to_vec(),
            pk_node_hash,
            agg_pk_commitment,
            signer_bitmap.to_vec(),
        )
    }
}

impl ConstraintSynthesizer<MNT6Fq> for PKTreeLeafCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        assert_eq!(self.pks.len(), self.signer_bitmap_chunk.len());

        // Allocate all the constants.
        let pedersen_generators_var =
            DefaultPedersenParametersVar::new_constant(cs.clone(), pedersen_parameters_mnt6())?;

        // Allocate all the witnesses.
        let pks_var = Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.pks[..]))?;

        // Allocate all the inputs.
        let pk_node_hash_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.pk_node_hash[..])?;

        let agg_pk_commitment_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.agg_pk_commitment[..])?;

        let signer_bitmap_chunk_bits =
            BitVec::<MNT6Fq>::new_input_vec(cs.clone(), &self.signer_bitmap_chunk)?;
        let signer_bitmap_chunk_bits =
            signer_bitmap_chunk_bits.0[..self.signer_bitmap_chunk.len()].to_vec();

        // Calculate the leaf hash and match it against the expected output.
        let mut bytes = vec![];
        for item in pks_var.iter() {
            bytes.extend(item.serialize_compressed(cs.clone())?);
        }

        let leaf_hash_bytes = evaluate_blake2s(&bytes)?;
        leaf_hash_bytes.enforce_equal(&pk_node_hash_bytes)?;

        // Calculate the aggregate public key.
        let mut calculated_agg_pk = G2Var::zero();

        for (pk, included) in pks_var.iter().zip(signer_bitmap_chunk_bits.iter()) {
            // Calculate a new sum that includes the next public key.
            let new_sum = &calculated_agg_pk + pk;

            // Choose either the new public key sum or the old public key sum, depending on whether
            // the bitmap indicates that the validator signed or not.
            let cond_sum =
                CondSelectGadget::conditionally_select(included, &new_sum, &calculated_agg_pk)?;

            calculated_agg_pk = cond_sum;
        }

        // Verifying aggregate public key. It checks that the calculated aggregate public key
        // is correct by comparing it with the aggregate public key commitment given as an input.
        let agg_pk_bytes = calculated_agg_pk.serialize_compressed(cs.clone())?;

        let pedersen_hash =
            DefaultPedersenHashGadget::evaluate(&agg_pk_bytes, &pedersen_generators_var)?;
        let pedersen_bytes = pedersen_hash.serialize_compressed(cs)?;

        agg_pk_commitment_bytes.enforce_equal(&pedersen_bytes)?;

        Ok(())
    }
}

impl Distribution<PKTreeLeafCircuit> for Standard {
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> PKTreeLeafCircuit {
        PKTreeLeafCircuit::sample_with_len(rng, Policy::SLOTS as usize / PK_TREE_BREADTH)
    }
}
