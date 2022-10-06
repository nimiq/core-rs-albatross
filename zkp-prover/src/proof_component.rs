use std::sync::Arc;

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};
use nimiq_block::MacroBlock;
use nimiq_hash::Blake2bHash;
use nimiq_nano_primitives::MacroBlock as ZKPMacroBlock;
use parking_lot::RwLock;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_database::{Database, Environment, ReadTransaction, WriteTransaction};
use nimiq_nano_zkp::{NanoZKP, NanoZKPError};
use tokio::sync::oneshot::Sender;

use super::types::ZKPState;
use crate::types::*;

pub(crate) fn pre_proof_validity_checks(
    blockchain: &Arc<RwLock<Blockchain>>,
    proof: ZKProof,
) -> Result<(MacroBlock, MacroBlock, Proof<MNT6_753>), ZKPComponentError> {
    let proof_block_hash = proof.header_hash;
    let proof = proof.proof.ok_or(NanoZKPError::EmptyProof)?;

    let new_block = blockchain
        .read()
        .get_block(&proof_block_hash, true, None)
        .ok_or(ZKPComponentError::InvalidBlock)?;

    if !new_block.is_election() {
        return Err(ZKPComponentError::InvalidBlock);
    }
    let new_block = new_block.unwrap_macro();

    let previous_hash = new_block.header.parent_election_hash.clone();
    let old_block = blockchain
        .read()
        .get_block(&previous_hash, true, None)
        .ok_or(ZKPComponentError::InvalidBlock)?;

    if !old_block.is_election() {
        return Err(ZKPComponentError::InvalidBlock);
    }
    let old_block = old_block.unwrap_macro();

    Ok((new_block, old_block, proof))
}

pub fn validate_proof(blockchain: &Arc<RwLock<Blockchain>>, proof: ZKProof) -> bool {
    if let Ok((new_block, old_block, proof)) = pre_proof_validity_checks(blockchain, proof) {
        return validate_proof_get_new_state(proof, new_block, old_block)
            .map_or(false, |zkp_state| zkp_state.is_some());
    }
    false
}

pub(crate) fn validate_proof_get_new_state(
    proof: Proof<MNT6_753>,
    new_block: MacroBlock,
    old_block: MacroBlock,
) -> Result<Option<ZKPState>, ZKPComponentError> {
    let old_pks: Vec<G2MNT6> = old_block
        .get_validators()
        .unwrap()
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect(); //itodo
    let new_pks: Vec<G2MNT6> = new_block
        .get_validators()
        .unwrap()
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect(); //itodo

    if NanoZKP::verify(
        old_block.block_number(),
        old_block.hash().into(),
        old_pks,
        new_block.block_number(),
        new_block.hash().into(),
        new_pks.clone(),
        proof.clone(),
    )? {
        return Ok(Some(ZKPState {
            latest_pks: new_pks,
            latest_header_hash: new_block.hash(),
            latest_block_number: new_block.block_number(),
            latest_proof: Some(proof),
        }));
    }
    Ok(None)
}

pub(crate) async fn generate_new_proof(
    block: MacroBlock,
    latest_pks: Vec<G2MNT6>,
    latest_header_hash: [u8; 32],
    previous_proof: Option<Proof<MNT6_753>>,
    genesis_state: Vec<u8>,
    sender: Sender<(Blake2bHash, Result<ZKPState, ZKPComponentError>)>,
) {
    let final_pks: Vec<G2MNT6> = block
        .get_validators()
        .unwrap()
        .voting_keys()
        .into_iter()
        .map(|pub_key| pub_key.public_key)
        .collect(); //itodo

    let block = ZKPMacroBlock::try_from(&block).unwrap();

    let proof = NanoZKP::prove(
        latest_pks.clone(),
        latest_header_hash,
        final_pks,
        block.clone(),
        previous_proof.map(|proof| (proof, genesis_state.clone())),
        true,
        true,
    );

    let new_block_blake_hash: Blake2bHash = block.header_hash.into();
    let result = match proof {
        Ok(proof) => sender.send((
            new_block_blake_hash.clone(),
            Ok(ZKPState {
                latest_pks,
                latest_header_hash: new_block_blake_hash,
                latest_block_number: block.block_number,
                latest_proof: Some(proof),
            }),
        )),
        Err(e) => sender.send((new_block_blake_hash, Err(ZKPComponentError::from(e)))),
    };

    if result.is_err() {
        log::error!(error = "receiver hung up", "error sending the zk proof",);
    }
}

#[derive(Debug)]
pub struct ProofStore {
    env: Environment,
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

    pub fn get_zkp(&self) -> Option<ZKPState> {
        ReadTransaction::new(&self.env).get(&self.zkp_db, ProofStore::PROOF_KEY)
    }

    pub fn set_zkp(&self, zkp_state: &ZKPState) {
        WriteTransaction::new(&self.env).put(&self.zkp_db, ProofStore::PROOF_KEY, zkp_state);
    }
}
