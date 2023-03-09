use ark_mnt6_753::{
    constraints::{FqVar, G1Var, G2Var},
    Fq as MNT6Fq, G1Projective, G2Projective,
};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean, CondSelectGadget, CurveVar, EqGadget, ToBitsGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::{PEDERSEN_PARAMETERS, PK_TREE_BREADTH, PK_TREE_DEPTH};

use crate::gadgets::{
    bits::BitVec,
    mnt6::{DefaultPedersenHashGadget, DefaultPedersenParametersVar, MerkleTreeGadget},
    serialize::SerializeGadget,
};

/// This is the leaf subcircuit of the PKTreeCircuit. This circuit main function is to process the
/// validator's public keys and "return" the aggregate public key for the Macro Block. At a
/// high-level, it divides all the computation into 2^n parts, where n is the depth of the tree, so
/// that each part uses only a manageable amount of memory and can be run on consumer hardware.
/// It does this by forming a binary tree of recursive SNARKs. Each of the 2^n leaves receives
/// Merkle tree commitments to the public keys list and a commitment to the corresponding aggregate
/// public key chunk (there are 2^n chunks, one for each leaf) in addition the position of the leaf
/// node on the tree (in little endian bits) and the part of the signer's bitmap relevant to the
/// leaf position. Each of the leaves then checks that its specific chunk of the public keys,
/// aggregated according to its specific chunk of the signer's bitmap, matches the corresponding
/// chunk of the aggregated public key.
/// All of the other upper levels of the recursive SNARK tree just verify SNARK proofs for its child
/// nodes and recursively aggregate the aggregate public key chunks (no pun intended).
/// At a lower-level, this circuit does two things:
///     1. That the public keys given as witness are a leaf of the Merkle tree of public keys, in a
///        specific position. The Merkle tree root and the position are given as inputs and the
///        Merkle proof is given as a witness.
///     2. That the public keys given as witness, when aggregated according to the signer's bitmap
///        (given as an input), match the aggregated public key commitment (also given as an input).
#[derive(Clone)]
pub struct PKTreeLeafCircuit {
    // Witnesses (private)
    pks: Vec<G2Projective>,
    pk_tree_nodes: Vec<G1Projective>,

    // Inputs (public)
    pk_tree_root: [u8; 95],
    agg_pk_commitment: [u8; 95],
    signer_bitmap_chunk: Vec<bool>,
    path: u8,
}

impl PKTreeLeafCircuit {
    pub fn new(
        pks: Vec<G2Projective>,
        pk_tree_nodes: Vec<G1Projective>,
        pk_tree_root: [u8; 95],
        agg_pk_commitment: [u8; 95],
        signer_bitmap: Vec<bool>,
        path: u8,
    ) -> Self {
        Self {
            pks,
            pk_tree_nodes,
            pk_tree_root,
            agg_pk_commitment,
            signer_bitmap_chunk: signer_bitmap,
            path,
        }
    }
}

impl ConstraintSynthesizer<MNT6Fq> for PKTreeLeafCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        // Allocate all the constants.
        let pedersen_generators_var =
            DefaultPedersenParametersVar::new_constant(cs.clone(), &*PEDERSEN_PARAMETERS)?; // only need 195

        // Allocate all the witnesses.
        let pks_var = Vec::<G2Var>::new_witness(cs.clone(), || Ok(&self.pks[..]))?;

        let pk_tree_nodes_var =
            Vec::<G1Var>::new_witness(cs.clone(), || Ok(&self.pk_tree_nodes[..]))?;

        // Allocate all the inputs.
        let pk_tree_root_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.pk_tree_root[..])?;

        let agg_pk_commitment_bytes =
            UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &self.agg_pk_commitment[..])?;

        let signer_bitmap_chunk_bits =
            BitVec::<MNT6Fq>::new_input_vec(cs.clone(), &self.signer_bitmap_chunk)?;
        let signer_bitmap_chunk_bits =
            signer_bitmap_chunk_bits.0[..Policy::SLOTS as usize / PK_TREE_BREADTH].to_vec();

        let path_var = FqVar::new_input(cs.clone(), || Ok(MNT6Fq::from(self.path)))?;

        // Unpack the inputs by converting them from field elements to bits and truncating appropriately.
        let path_bits = path_var.to_bits_le()?[..PK_TREE_DEPTH].to_vec();

        // Verify the Merkle proof for the public keys. To reiterate, this Merkle tree has 2^n
        // leaves (where n is the PK_TREE_DEPTH constant) and each of the leaves consists of several
        // public keys serialized and concatenated together. Each leaf contains exactly
        // VALIDATOR_SLOTS/2^n public keys, so that the entire Merkle tree contains all of the
        // public keys.
        let mut bytes = vec![];

        for item in pks_var.iter().take(self.pks.len()) {
            bytes.extend(item.serialize_compressed(cs.clone())?);
        }

        MerkleTreeGadget::verify(
            cs.clone(),
            &bytes,
            &pk_tree_nodes_var,
            &path_bits,
            &pk_tree_root_bytes,
            &pedersen_generators_var,
        )?
        .enforce_equal(&Boolean::constant(true))?;

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
