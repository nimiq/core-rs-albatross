use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
};

use ark_groth16::Proof;
use ark_mnt6_753::MNT6_753;
use nimiq_block::MacroBlock;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_zkp::prove::prove;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::oneshot::Receiver,
};

use super::types::ZKPState;
use crate::types::*;

/// Generates the zk proof and sends it through the channel provided. Upon failure, the error is sent through the channel provided.
pub fn generate_new_proof(
    prev_block: MacroBlock,
    previous_proof: Option<Proof<MNT6_753>>,
    final_block: MacroBlock,
    genesis_header_hash: [u8; 32],
    prover_keys_path: &Path,
) -> Result<ZKPState, ZKProofGenerationError> {
    let proof = prove(
        prev_block,
        final_block.clone(),
        previous_proof.map(|proof| (proof, genesis_header_hash)),
        true,
        true,
        prover_keys_path,
    );

    match proof {
        Ok(proof) => Ok(ZKPState {
            latest_block: final_block,
            latest_proof: Some(proof),
        }),
        Err(e) => {
            log::error!("Encountered error during proving: {:?}", e);
            Err(ZKProofGenerationError::from(e))
        }
    }
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
        None => env::current_exe()?,
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
            parse_proof_generation_output(buffer)
        }
        _ = recv => {
            child.kill().await?;
            Err(ZKProofGenerationError::ChannelError)
        },
    }
}

fn parse_proof_generation_output(output: Vec<u8>) -> Result<ZKPState, ZKProofGenerationError> {
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
