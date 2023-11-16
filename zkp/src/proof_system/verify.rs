use ark_crypto_primitives::snark::SNARK;
use ark_ec::mnt6::MNT6;
use ark_ff::ToConstraintField;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::{Config, MNT6_753};
use nimiq_hash::Blake2sHash;
use nimiq_zkp_primitives::{vk_commitment, NanoZKPError};

/// This function verifies a proof for the Merger Wrapper circuit, which implicitly is a proof for
/// the entire light macro sync. It is very fast, shouldn't take more than a second, even on older
/// computers.
pub fn verify(
    // The header hash of the initial block.
    genesis_header_hash: Blake2sHash,
    // The header hash of the final block.
    final_header_hash: Blake2sHash,
    // The SNARK proof for this circuit.
    proof: Proof<MNT6_753>,
    verifying_key: &VerifyingKey<MNT6<Config>>,
) -> Result<bool, NanoZKPError> {
    // Prepare the inputs.
    let mut inputs = vec![];

    inputs.append(&mut genesis_header_hash.0.to_field_elements().unwrap());
    inputs.append(&mut final_header_hash.0.to_field_elements().unwrap());

    inputs.append(&mut vk_commitment(&verifying_key).to_field_elements().unwrap());

    // Verify proof.
    let result = Groth16::<MNT6_753>::verify(verifying_key, &inputs, &proof)?;

    // Return result.
    Ok(result)
}
