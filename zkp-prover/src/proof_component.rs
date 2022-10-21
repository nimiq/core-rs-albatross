use std::sync::Arc;

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};
use nimiq_block::{Block, MacroBlock};
use nimiq_genesis::NetworkInfo;
use nimiq_nano_primitives::MacroBlock as ZKPMacroBlock;
use parking_lot::RwLock;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_database::{Database, Environment, ReadTransaction, WriteTransaction};
use nimiq_nano_zkp::{NanoZKP, NanoZKPError};
use tokio::sync::oneshot::Sender;

use super::types::ZKPState;
use crate::types::*;

pub fn validate_proof(blockchain: &Arc<RwLock<Blockchain>>, proof: &ZKProof) -> bool {
    // If it's a genesis block proof, then should have none as proof value to be valid
    if proof.block_number == 0 && proof.proof.is_none() {
        return true;
    }

    if let Ok((new_block, genesis_block, proof)) = pre_proof_validity_checks(blockchain, proof) {
        return validate_proof_get_new_state(proof, new_block, genesis_block).is_ok();
    }
    false
}

pub(crate) fn pre_proof_validity_checks(
    blockchain: &Arc<RwLock<Blockchain>>,
    proof: &ZKProof,
) -> Result<(MacroBlock, MacroBlock, Proof<MNT6_753>), ZKPComponentError> {
    let block_number = proof.block_number;
    let proof = proof.proof.clone().ok_or(NanoZKPError::EmptyProof)?;

    let new_block = blockchain
        .read()
        .get_block_at(block_number, true, None)
        .ok_or(ZKPComponentError::InvalidBlock)?;

    if !new_block.is_election() {
        return Err(ZKPComponentError::InvalidBlock);
    }
    let new_block = new_block.unwrap_macro();

    let network_info = NetworkInfo::from_network_id(blockchain.read().network_id());
    let genesis_block = network_info.genesis_block::<Block>().unwrap_macro();

    Ok((new_block, genesis_block, proof))
}

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

pub(crate) async fn generate_new_proof(
    block: MacroBlock,
    latest_pks: Vec<G2MNT6>,
    latest_header_hash: [u8; 32],
    previous_proof: Option<Proof<MNT6_753>>,
    genesis_state: Vec<u8>,
    sender: Sender<Result<ZKPState, ZKPComponentError>>,
) {
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

        let result = match proof {
            Ok(proof) => sender.send(Ok(ZKPState {
                latest_pks,
                latest_header_hash: block.header_hash.into(),
                latest_block_number: block.block_number,
                latest_proof: Some(proof),
            })),
            Err(e) => sender.send(Err(ZKPComponentError::from(e))),
        };

        if result.is_err() {
            log::error!(error = "receiver hung up", "error sending the zk proof",);
        }
    } else if sender.send(Err(ZKPComponentError::InvalidBlock)).is_err() {
        log::error!(error = "receiver hung up", "error sending the zk proof",);
    }
}

#[derive(Debug)]
pub struct ProofStore {
    pub env: Environment,
    // A database of the curreent zkp state indexed by their block number.
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
