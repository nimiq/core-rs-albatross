#![feature(async_closure)]
#![feature(drain_filter)]

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;

extern crate nimiq_macros as macros;
extern crate nimiq_network_interface as network_interface;
extern crate nimiq_utils as utils;

use crate::tendermint_state::TendermintStateProof;
use beserial::{Deserialize, Serialize};

pub(crate) mod protocol;
pub(crate) mod tendermint_outside_deps;
pub(crate) mod tendermint_state;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Step {
    Propose,
    Prevote,
    Precommit,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SingleDecision {
    Block,
    Nil,
}

pub enum ProposalResult<ProposalTy> {
    // value + valid round
    // round is not necessary since we only receive proposals for the current round. but might add it.
    Proposal(ProposalTy, Option<u32>),
    Timeout,
}

pub enum AggregationResult<ProofTy> {
    // might need to have round and value hash.
    Block(ProofTy),
    Nil(ProofTy),
    Timeout(ProofTy),
}

pub enum TendermintReturn<ProposalTy, ProofTy, ResultTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
{
    Result(ResultTy),
    StateUpdate(TendermintStateProof<ProposalTy, ProofTy>),
    Error(TendermintError),
}

pub enum TendermintError {
    NetworkEnded,
    BadInitState,
}
