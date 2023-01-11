use std::path::Path;

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};

use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database::{Database, Environment, ReadTransaction, WriteTransaction};
use nimiq_genesis::NetworkInfo;
use nimiq_nano_zkp::{NanoZKP, NanoZKPError};

use super::types::ZKPState;
use crate::types::*;

/// Fully validates the proof by verifying both the zk proof and the blocks existence on the blockchain.
pub fn validate_proof(
    blockchain: &BlockchainProxy,
    proof: &ZKProof,
    election_block: Option<MacroBlock>,
    keys_path: &Path,
) -> bool {
    // If it's a genesis block proof, then should have none as proof value to be valid.
    if proof.block_number == 0 && proof.proof.is_none() {
        return true;
    }

    // Fetches and verifies the election blocks for the proofs and then validates the proof
    if let Ok((new_block, genesis_block, proof)) =
        get_proof_macro_blocks(blockchain, proof, election_block)
    {
        return validate_proof_get_new_state(proof, &new_block, genesis_block, keys_path).is_ok();
    }
    false
}

/// Fetches both the genesis and the proof block. Fails if the proof block is not an election block or the proof is empty.
/// If the given option already contains the new election block, we return it. Otherwise, we retrieve it from the blockchain.
pub(crate) fn get_proof_macro_blocks(
    blockchain: &BlockchainProxy,
    proof: &ZKProof,
    election_block: Option<MacroBlock>,
) -> Result<(MacroBlock, MacroBlock, Proof<MNT6_753>), Error> {
    let block_number = proof.block_number;
    let proof = proof.proof.clone().ok_or(NanoZKPError::EmptyProof)?;

    // Gets the block of the new proof
    let new_block = if let Some(block) = election_block {
        block
    } else {
        let new_block = blockchain
            .read()
            .get_block_at(block_number, true)
            .map_err(|_| Error::InvalidBlock)?;

        if !new_block.is_election() {
            return Err(Error::InvalidBlock);
        }
        new_block.unwrap_macro()
    };

    // Gets genesis block
    let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
    let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();

    Ok((new_block, genesis_block, proof))
}

/// Validates proof and returns the new zkp state. Assumes the blocks provided are valid.
pub(crate) fn validate_proof_get_new_state(
    proof: Proof<MNT6_753>,
    new_block: &MacroBlock,
    genesis_block: MacroBlock,
    keys_path: &Path,
) -> Result<ZKPState, Error> {
    let genesis_pks: Vec<G2MNT6> = genesis_block
        .get_validators()
        .ok_or(Error::InvalidBlock)?
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect();
    let new_pks: Vec<G2MNT6> = new_block
        .get_validators()
        .ok_or(Error::InvalidBlock)?
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
        keys_path,
    )? {
        return Ok(ZKPState {
            latest_pks: new_pks,
            latest_header_hash: new_block.hash(),
            latest_block_number: new_block.block_number(),
            latest_proof: Some(proof),
        });
    }
    Err(Error::InvalidProof)
}

/// DB for storing and retrieving the ZK Proof.
#[derive(Debug)]
pub struct ProofStore {
    pub env: Environment,
    // A database of the current zkp state.
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
