use ark_crypto_primitives::snark::SNARK;
use ark_ff::ToConstraintField;
use ark_groth16::{Groth16, Proof};
use ark_mnt6_753::MNT6_753;
use nimiq_hash::Blake2sHash;
use nimiq_zkp_primitives::{NanoZKPError, VerifyingData};

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
    verifying_data: &VerifyingData,
) -> Result<bool, NanoZKPError> {
    // Prepare the inputs.
    let mut inputs = vec![];

    inputs.append(&mut genesis_header_hash.0.to_field_elements().unwrap());
    inputs.append(&mut final_header_hash.0.to_field_elements().unwrap());
    inputs.append(&mut verifying_data.keys_commitment.to_field_elements().unwrap());

    // Verify proof.
    let result = Groth16::<MNT6_753>::verify(&verifying_data.merger_wrapper_vk, &inputs, &proof)?;

    // Return result.
    Ok(result)
}
