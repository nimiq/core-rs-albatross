use std::{process::Stdio, sync::Arc};

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};
use beserial::{Deserialize, Serialize};
use nimiq_block::{Block, MacroBlock};
use nimiq_genesis::NetworkInfo;
use nimiq_nano_primitives::MacroBlock as ZKPMacroBlock;
use parking_lot::RwLock;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_database::{Database, Environment, ReadTransaction, WriteTransaction};
use nimiq_nano_zkp::{NanoZKP, NanoZKPError};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::sync::broadcast::Receiver as BroadcastReceiver;
use tokio::{io::AsyncWriteExt, process::Command};

use super::types::ZKPState;
use crate::types::*;

/// Fully validates the proof by verifying both the zk proof and the blocks existance on the blockchain.
pub fn validate_proof(blockchain: &Arc<RwLock<Blockchain>>, proof: &ZKProof) -> bool {
    // If it's a genesis block proof, then should have none as proof value to be valid.
    if proof.block_number == 0 && proof.proof.is_none() {
        return true;
    }

    // Fetches and verfies the election blocks for the proofs and then validates the proof
    if let Ok((new_block, genesis_block, proof)) = get_proof_macro_blocks(blockchain, proof) {
        return validate_proof_get_new_state(proof, new_block, genesis_block).is_ok();
    }
    false
}

/// Fetches both the genesis and the proof block. Fails if the proof block is a election block.
pub(crate) fn get_proof_macro_blocks(
    blockchain: &Arc<RwLock<Blockchain>>,
    proof: &ZKProof,
) -> Result<(MacroBlock, MacroBlock, Proof<MNT6_753>), ZKPComponentError> {
    let block_number = proof.block_number;
    let proof = proof.proof.clone().ok_or(NanoZKPError::EmptyProof)?;

    // Gets the block of the new proof
    let new_block = blockchain
        .read()
        .get_block_at(block_number, true, None)
        .ok_or(ZKPComponentError::InvalidBlock)?;

    if !new_block.is_election() {
        return Err(ZKPComponentError::InvalidBlock);
    }
    let new_block = new_block.unwrap_macro();

    // Gets genesis block
    let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
    let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();

    Ok((new_block, genesis_block, proof))
}

/// Validates proof and returns the new zkp state. Assumes the blocks provided are valid.
pub(crate) fn validate_proof_get_new_state(
    proof: Proof<MNT6_753>,
    new_block: MacroBlock,
    genesis_block: MacroBlock,
) -> Result<ZKPState, ZKPComponentError> {
    let genesis_pks: Vec<G2MNT6> = genesis_block
        .get_validators()
        .ok_or(ZKPComponentError::InvalidBlock)?
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect();
    let new_pks: Vec<G2MNT6> = new_block
        .get_validators()
        .ok_or(ZKPComponentError::InvalidBlock)?
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect();

    if NanoZKP::verify(
        genesis_block.block_number(),
        genesis_block.hash().into(),
        genesis_pks,
        new_block.block_number(),
        new_block.hash().into(),
        new_pks.clone(),
        proof.clone(),
    )? {
        return Ok(ZKPState {
            latest_pks: new_pks,
            latest_header_hash: new_block.hash(),
            latest_block_number: new_block.block_number(),
            latest_proof: Some(proof),
        });
    }
    Err(ZKPComponentError::InvalidProof)
}

/// Generates the zk proof and sends it through the channel provided. Upon failure, the error is sent trough the channel providded.
pub fn generate_new_proof(
    block: MacroBlock,
    latest_pks: Vec<G2MNT6>,
    latest_header_hash: [u8; 32],
    previous_proof: Option<Proof<MNT6_753>>,
    genesis_state: Vec<u8>,
) -> Result<ZKPState, ZKProofGenerationError> {
    let validators = block.get_validators();

    if let Some(validators) = validators {
        let final_pks = validators
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect();

        let block = ZKPMacroBlock::try_from(&block).expect("Invalid election block");

        let proof = NanoZKP::prove(
            latest_pks.clone(),
            latest_header_hash,
            final_pks,
            block.clone(),
            previous_proof.map(|proof| (proof, genesis_state.clone())),
            true,
            true,
        );

        return match proof {
            Ok(proof) => Ok(ZKPState {
                latest_pks,
                latest_header_hash: block.header_hash.into(),
                latest_block_number: block.block_number,
                latest_proof: Some(proof),
            }),
            Err(e) => Err(ZKProofGenerationError::from(e)),
        };
    }
    Err(ZKProofGenerationError::InvalidBlock)
}

pub(crate) async fn launch_generate_new_proof(
    mut recv: BroadcastReceiver<()>,
    proof_input: ProofInput,
) -> Result<ZKPState, ZKProofGenerationError> {
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--prove")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(&proof_input.serialize_to_vec())
        .await?;

    tokio::select! {
        _ = child.wait() => {
            let output = child.stdout.take().unwrap();
            parse_proof_generation_output(output).await
        }
        _ = recv.recv() => {
            child.kill().await?;
            Err(ZKProofGenerationError::ChannelError)
        },
    }
}

async fn parse_proof_generation_output(
    output: ChildStdout,
) -> Result<ZKPState, ZKProofGenerationError> {
    let mut reader = BufReader::new(output);
    // We read until the two delimiter bytes.
    // `prev_byte` stores the last read byte.
    let mut prev_byte = 0;
    loop {
        // If the previous byte is not the first delimiter byte, we skip until we encounter it.
        if prev_byte != PROOF_GENERATION_OUTPUT_DELIMITER[0] {
            let mut buffer = vec![];
            reader
                .read_until(PROOF_GENERATION_OUTPUT_DELIMITER[0], &mut buffer)
                .await?;
        }

        // Then attempt to read another byte.
        prev_byte = reader.read_u8().await?;
        // If the byte is the expected second byte, break.
        if prev_byte == PROOF_GENERATION_OUTPUT_DELIMITER[1] {
            break;
        }
    }
    let mut buffer = vec![];
    reader.read_to_end(&mut buffer).await?;

    Deserialize::deserialize_from_vec(&buffer)?
}

/// DB for storing and retrieving the ZK Proof.
#[derive(Debug)]
pub struct ProofStore {
    pub env: Environment,
    // A database of the curreent zkp state.
    zkp_db: Database,
}

impl ProofStore {
    const PROOF_DB_NAME: &'static str = "ZKPState";
    const PROOF_KEY: &'static str = "proof";

    pub fn new(env: Environment) -> Self {
        let zkp_db = env.open_database(Self::PROOF_DB_NAME.to_string());

        ProofStore { env, zkp_db }
    }

    pub fn get_zkp(&self) -> Option<ZKProof> {
        ReadTransaction::new(&self.env).get(&self.zkp_db, ProofStore::PROOF_KEY)
    }

    pub fn set_zkp(&self, zk_proof: &ZKProof) {
        let mut tx = WriteTransaction::new(&self.env);
        tx.put(&self.zkp_db, ProofStore::PROOF_KEY, zk_proof);
        tx.commit();
    }
}
