use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::oneshot::Receiver,
};

use beserial::{Deserialize, Serialize};
use nimiq_block::MacroBlock;
use nimiq_zkp::prove::prove;
use nimiq_zkp_primitives::MacroBlock as ZKPMacroBlock;

use super::types::ZKPState;
use crate::types::*;

/// Generates the zk proof and sends it through the channel provided. Upon failure, the error is sent trough the channel provided.
pub fn generate_new_proof(
    block: MacroBlock,
    prev_pks: Vec<G2MNT6>,
    prev_header_hash: [u8; 32],
    previous_proof: Option<Proof<MNT6_753>>,
    genesis_state: [u8; 95],
    prover_keys_path: &Path,
) -> Result<ZKPState, ZKProofGenerationError> {
    let validators = block.get_validators();

    if let Some(validators) = validators {
        let final_pks: Vec<_> = validators
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect();

        let block = ZKPMacroBlock::try_from(&block).expect("Invalid election block");

        let proof = prove(
            prev_pks,
            prev_header_hash,
            final_pks.clone(),
            block.clone(),
            previous_proof.map(|proof| (proof, genesis_state)),
            true,
            true,
            prover_keys_path,
        );

        return match proof {
            Ok(proof) => Ok(ZKPState {
                latest_pks: final_pks,
                latest_header_hash: block.header_hash.into(),
                latest_block_number: block.block_number,
                latest_proof: Some(proof),
            }),
            Err(e) => {
                log::error!("Encountered error during proving: {:?}", e);
                Err(ZKProofGenerationError::from(e))
            }
        };
    }
    Err(ZKProofGenerationError::InvalidBlock)
}

/// Starts the prover in a new child process.
/// Warning: The child process will continue to run if the parent process crashes.
pub async fn launch_generate_new_proof(
    recv: Receiver<()>,
    proof_input: ProofInput,
    prover_path: Option<PathBuf>,
) -> Result<ZKPState, ZKProofGenerationError> {
    let path = match prover_path {
        Some(path) => path,
        None => std::env::current_exe()?,
    };
    log::debug!("Launching the prover process at path {:?}", path);

    let mut child = Command::new(path)
        .arg("--prove")
        .args(env::args().skip(1))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(&proof_input.serialize_to_vec())
        .await?;

    let mut output = child.stdout.take().unwrap();
    let mut buffer = vec![];

    tokio::select! {
        _ = output.read_to_end(&mut buffer) => {
            let _ = child.wait().await;
            parse_proof_generation_output(buffer).await
        }
        _ = recv => {
            child.kill().await?;
            Err(ZKProofGenerationError::ChannelError)
        },
    }
}

async fn parse_proof_generation_output(
    output: Vec<u8>,
) -> Result<ZKPState, ZKProofGenerationError> {
    // We need at least two bytes to proceed.
    if output.len() < 2 {
        return Err(ZKProofGenerationError::ProcessError(
            "Did not receive enough output from process".to_owned(),
        ));
    }

    // We read until the two delimiter bytes.
    let mut mid = 0;
    for i in 0..output.len() - 2 {
        if output[i..i + 2] == PROOF_GENERATION_OUTPUT_DELIMITER {
            mid = i + 2;
            break;
        }
    }

    Deserialize::deserialize_from_vec(&output[mid..])?
}
