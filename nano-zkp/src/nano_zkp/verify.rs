use std::fs::File;

use ark_crypto_primitives::SNARK;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::{G1Projective as G1MNT6, MNT6_753};
use ark_serialize::CanonicalDeserialize;

use nimiq_bls::utils::bytes_to_bits;
use nimiq_nano_primitives::{state_commitment, vk_commitment};

use crate::utils::pack_inputs;
use crate::{NanoZKP, NanoZKPError};

impl NanoZKP {
    /// This function verifies a proof for the Merger Wrapper circuit, which implicitly is a proof for
    /// the entire nano sync program. It is very fast, shouldn't take more than a second, even on older
    /// computers.
    pub fn verify(
        // The block number of the initial block. Most likely, it will be the genesis block.
        initial_block_number: u32,
        // The header hash of the initial block. Most likely, it will be the genesis block.
        initial_header_hash: [u8; 32],
        // The public keys of the validators of the initial block. Most likely, it will be the genesis
        // block.
        // Note that we are referring to the validators that are selected in the initial block, not
        // the validators that signed the initial block.
        initial_pks: Vec<G1MNT6>,
        // The block number of the final block.
        final_block_number: u32,
        // The header hash of the final block.
        final_header_hash: [u8; 32],
        // The public keys of the validators of the final block.
        // Note that we are referring to the validators that are selected in the final block, not
        // the validators that signed the final block.
        final_pks: Vec<G1MNT6>,
        // The SNARK proof for this circuit.
        proof: Proof<MNT6_753>,
    ) -> Result<bool, NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/merger_wrapper.bin")?;

        let vk = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];

        inputs.append(&mut pack_inputs(bytes_to_bits(&state_commitment(
            initial_block_number,
            initial_header_hash,
            initial_pks,
        ))));

        inputs.append(&mut pack_inputs(bytes_to_bits(&state_commitment(
            final_block_number,
            final_header_hash,
            final_pks,
        ))));

        inputs.append(&mut pack_inputs(bytes_to_bits(&vk_commitment(vk.clone()))));

        // Verify proof.
        let result = Groth16::<MNT6_753>::verify(&vk, &inputs, &proof)?;

        // Return result.
        Ok(result)
    }
}
