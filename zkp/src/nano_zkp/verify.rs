use ark_crypto_primitives::SNARK;
use ark_ec::mnt6::MNT6;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::{G2Projective as G2MNT6, Parameters, MNT6_753};

use nimiq_bls::utils::bytes_to_bits;
use nimiq_zkp_circuits::utils::pack_inputs;
use nimiq_zkp_primitives::{state_commitment, vk_commitment, NanoZKPError};

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
    initial_pks: Vec<G2MNT6>,
    // The block number of the final block.
    final_block_number: u32,
    // The header hash of the final block.
    final_header_hash: [u8; 32],
    // The public keys of the validators of the final block.
    // Note that we are referring to the validators that are selected in the final block, not
    // the validators that signed the final block.
    final_pks: Vec<G2MNT6>,
    // The SNARK proof for this circuit.
    proof: Proof<MNT6_753>,
    verifying_key: &VerifyingKey<MNT6<Parameters>>,
) -> Result<bool, NanoZKPError> {
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

    inputs.append(&mut pack_inputs(bytes_to_bits(&vk_commitment(
        verifying_key.clone(),
    ))));

    // Verify proof.
    let result = Groth16::<MNT6_753>::verify(verifying_key, &inputs, &proof)?;

    // Return result.
    Ok(result)
}
