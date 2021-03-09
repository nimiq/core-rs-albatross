use std::fs::File;

use ark_crypto_primitives::SNARK;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::MNT6_753;
use ark_serialize::CanonicalDeserialize;

use nimiq_nano_primitives::{bytes_to_bits, pack_inputs};

use crate::primitives::vk_commitment;
use crate::{NanoZKP, NanoZKPError};

impl NanoZKP {
    /// This function verifies a proof for the Merger Wrapper circuit, which implicitly is a proof for
    /// the entire nano sync program. It is very fast, shouldn't take more than a second, even on older
    /// computers.
    pub fn verify(
        // The state commitment for the initial state. It is composed of the public keys of the
        // validators and the block number of the initial block. Most likely, it will be the genesis
        // block.
        // Note that we are referring to the validators that are selected in the initial block, not
        // the validators that signed the initial block.
        initial_state_commitment: Vec<u8>,
        // The state commitment for the final state. It is composed of the public keys of the
        // validators and the block number of the final block.
        // Note that we are referring to the validators that are selected in the final block, not
        // the validators that signed the final block.
        final_state_commitment: Vec<u8>,
        // The SNARK proof for this circuit.
        proof: Proof<MNT6_753>,
    ) -> Result<bool, NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/merger_wrapper.bin".to_string())?;

        let vk = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];

        inputs.append(&mut pack_inputs(bytes_to_bits(&initial_state_commitment)));

        inputs.append(&mut pack_inputs(bytes_to_bits(&final_state_commitment)));

        inputs.append(&mut pack_inputs(bytes_to_bits(&vk_commitment(vk.clone()))));

        // Verify proof.
        let result = Groth16::<MNT6_753>::verify(&vk, &inputs, &proof)?;

        // Return result.
        Ok(result)
    }
}
