use std::{collections::BTreeMap, fmt::Debug, io};

use nimiq_block::{MacroBody, MacroHeader};
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::Blake2sHash;
use nimiq_keys::Ed25519Signature as SchnorrSignature;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_tendermint::{State as TendermintState, Step};
use nimiq_validator_network::{PubsubId, ValidatorNetwork};

use super::{
    contribution::TendermintContribution,
    proposal::{Body, Header, SignedProposal},
};
use crate::tendermint::TendermintProtocol;

#[derive(Clone, Serialize, Deserialize)]
pub struct MacroState {
    pub(crate) block_number: u32,
    round_number: u32,
    step: Step,
    known_proposals: BTreeMap<Blake2sHash, MacroHeader>,
    round_proposals: BTreeMap<u32, BTreeMap<Blake2sHash, (Option<u32>, (SchnorrSignature, u16))>>,
    votes: BTreeMap<(u32, Step), Option<Blake2sHash>>,
    best_votes: BTreeMap<(u32, Step), TendermintContribution>,
    inherents: BTreeMap<Blake2sHash, MacroBody>,
    locked: Option<(u32, Blake2sHash)>,
    valid: Option<(u32, Blake2sHash)>,
}

impl Debug for MacroState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("MacroState");
        dbg.field("round", &self.round_number);
        dbg.field("step", &self.step);
        dbg.field("locked", &self.locked);
        dbg.field("valid", &self.valid);
        dbg.field("best_votes", &self.best_votes);
        dbg.field("votes", &self.votes);

        dbg.finish()
    }
}

impl MacroState {
    pub fn from_tendermint_state<TValidatorNetwork>(
        block_number: u32,
        state: TendermintState<TendermintProtocol<TValidatorNetwork>>,
    ) -> Self
    where
        TValidatorNetwork: ValidatorNetwork + 'static,
        PubsubId<TValidatorNetwork>: Unpin,
    {
        let mut known_proposals = BTreeMap::default();
        for (proposal_hash, Header(proposal, _)) in state.known_proposals.into_iter() {
            known_proposals.insert(proposal_hash, proposal);
        }
        let mut inherents = BTreeMap::default();
        for (inherent_hash, Body(macro_body)) in state.inherents.into_iter() {
            inherents.insert(inherent_hash, macro_body);
        }

        Self {
            block_number,
            round_number: state.current_round,
            step: state.current_step,
            known_proposals,
            round_proposals: state.round_proposals,
            votes: state.votes,
            best_votes: state.best_votes,
            inherents,
            locked: state.locked,
            valid: state.valid,
        }
    }

    pub fn into_tendermint_state<TValidatorNetwork>(
        self,
        reference_height: u32,
    ) -> Option<TendermintState<TendermintProtocol<TValidatorNetwork>>>
    where
        TValidatorNetwork: ValidatorNetwork + 'static,
        PubsubId<TValidatorNetwork>: Unpin,
    {
        if self.block_number != reference_height {
            return None;
        }

        let mut known_proposals = BTreeMap::default();
        for (proposal_hash, proposal) in self.known_proposals.iter() {
            known_proposals.insert(
                proposal_hash.clone(),
                Header::<PubsubId<TValidatorNetwork>>(proposal.clone(), None),
            );
        }
        let mut inherents = BTreeMap::default();
        for (inherent_hash, inherent) in self.inherents.iter() {
            inherents.insert(inherent_hash.clone(), Body(inherent.clone()));
        }
        Some(TendermintState {
            current_round: self.round_number,
            current_step: self.step,
            known_proposals,
            round_proposals: self.round_proposals,
            votes: self.votes,
            best_votes: self.best_votes,
            inherents,
            locked: self.locked,
            valid: self.valid,
        })
    }

    pub fn get_proposal_for(
        &self,
        block_number: u32,
        round_number: u32,
        proposal_hash: &Blake2sHash,
    ) -> Option<SignedProposal> {
        if self.block_number != block_number {
            return None;
        }

        let (valid_round, signature) = self
            .round_proposals
            .get(&round_number)?
            .get(proposal_hash)?
            .clone();

        let proposal = self
            .known_proposals
            .get(proposal_hash)
            .expect("Proposal which exists in round_proposals must also exist in known_proposals.")
            .clone();

        Some(SignedProposal {
            proposal,
            round: round_number,
            valid_round,
            signature: signature.0,
            signer: signature.1,
        })
    }
}

impl IntoDatabaseValue for MacroState {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize_to_writer(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for MacroState {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Self::deserialize_from_vec(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}
