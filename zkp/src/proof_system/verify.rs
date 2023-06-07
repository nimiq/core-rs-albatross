use ark_crypto_primitives::snark::SNARK;
use ark_ec::mnt6::MNT6;
use ark_ff::ToConstraintField;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::{Config, MNT6_753};
use nimiq_zkp_primitives::{state_commitment, vk_commitment, NanoZKPError};

/// This function verifies a proof for the Merger Wrapper circuit, which implicitly is a proof for
/// the entire light macro sync. It is very fast, shouldn't take more than a second, even on older
/// computers.
pub fn verify(
    // The block number of the initial block.
    genesis_block_number: u32,
    // The header hash of the initial block.
    genesis_header_hash: [u8; 32],
    // The public key tree root of the validators of the initial block.
    // Note that we are referring to the validators that are selected in the initial block, not
    // the validators that signed the initial block.
    genesis_pk_tree_root: &[u8; 95],
    // The block number of the final block.
    final_block_number: u32,
    // The header hash of the final block.
    final_header_hash: [u8; 32],
    // The public keys of the validators of the final block.
    // Note that we are referring to the validators that are selected in the final block, not
    // the validators that signed the final block.
    final_pk_tree_root: &[u8; 95],
    // The SNARK proof for this circuit.
    proof: Proof<MNT6_753>,
    verifying_key: &VerifyingKey<MNT6<Config>>,
) -> Result<bool, NanoZKPError> {
    // Prepare the inputs.
    let mut inputs = vec![];

    inputs.append(
        &mut state_commitment(
            genesis_block_number,
            &genesis_header_hash,
            genesis_pk_tree_root,
        )
        .to_field_elements()
        .unwrap(),
    );

    inputs.append(
        &mut state_commitment(final_block_number, &final_header_hash, final_pk_tree_root)
            .to_field_elements()
            .unwrap(),
    );

    inputs.append(
        &mut vk_commitment(verifying_key.clone())
            .to_field_elements()
            .unwrap(),
    );

    // Verify proof.
    let result = Groth16::<MNT6_753>::verify(verifying_key, &inputs, &proof)?;

    // Return result.
    Ok(result)
}
