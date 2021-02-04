use std::fs::File;

use ark_crypto_primitives::SNARK;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};
use ark_serialize::CanonicalDeserialize;

use crate::primitives::{state_commitment, vk_commitment};
use crate::utils::{bytes_to_bits, prepare_inputs};
use crate::{NanoZKP, NanoZKPError};

impl NanoZKP {
    pub fn verify(
        initial_pks: Vec<G2MNT6>,
        initial_block_number: u32,
        final_pks: Vec<G2MNT6>,
        final_block_number: u32,
        proof: Proof<MNT6_753>,
    ) -> Result<bool, NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/merger_wrapper.bin"))?;

        let vk = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Prepare the inputs.
        let initial_state_commitment = state_commitment(initial_block_number, initial_pks);

        let final_state_commitment = state_commitment(final_block_number, final_pks);

        let mut inputs = vec![];

        inputs.append(&mut prepare_inputs(bytes_to_bits(
            &initial_state_commitment,
        )));

        inputs.append(&mut prepare_inputs(bytes_to_bits(&final_state_commitment)));

        inputs.append(&mut prepare_inputs(bytes_to_bits(&vk_commitment(
            vk.clone(),
        ))));

        // Verify proof.
        let result = Groth16::<MNT6_753>::verify(&vk, &inputs, &proof)?;

        // Return result.
        Ok(result)
    }
}
