use std::collections::BTreeMap;

use beserial::{Deserialize, Serialize};

use crate::{
    utils::{Checkpoint, Step},
    AggregationResult, ProofTrait, ProposalCacheTrait, ProposalHashTrait, ProposalTrait,
};

/// necessary for serialization/deserialization derivation
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Aggregate<ProposalHashTy: ProposalHashTrait, ProofTy: ProofTrait> {
    #[beserial(len_type(u16))]
    pub aggregate: BTreeMap<Option<ProposalHashTy>, (ProofTy, u16)>,
}

impl<ProposalHashTy: ProposalHashTrait, ProofTy: ProofTrait>
    From<AggregationResult<ProposalHashTy, ProofTy>> for Aggregate<ProposalHashTy, ProofTy>
{
    fn from(result: AggregationResult<ProposalHashTy, ProofTy>) -> Self {
        match result {
            AggregationResult::Aggregation(aggregate) => Self { aggregate },
            _ => panic!("Must not transform any other AggregateReturn into an Aggregate than AggregateReturn::Aggregate!"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExtendedProposal<ProposalTy: ProposalTrait, ProposalHashTy: ProposalHashTrait> {
    pub round: u32,
    pub proposal: ProposalTy,
    pub proposal_hash: ProposalHashTy,
}

/// The struct that stores Tendermint's state. It contains both the state variables describes in the
/// protocol (except for the height and the decision vector since we don't need them) and some extra
/// variables that we need for our implementation (those are prefixed with "current").
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TendermintState<
    ProposalTy: ProposalTrait,
    ProposalCacheTy: ProposalCacheTrait,
    ProposalHashTy: ProposalHashTrait,
    ProofTy: ProofTrait,
> {
    pub height: u32,
    pub round: u32,
    pub step: Step,
    pub locked: Option<ExtendedProposal<ProposalTy, ProposalHashTy>>,
    pub valid: Option<ExtendedProposal<ProposalTy, ProposalHashTy>>,

    pub current_checkpoint: Checkpoint,
    pub is_our_turn: Option<bool>,
    pub current_proposal: Option<Option<(ProposalTy, ProposalHashTy)>>,
    pub current_proposal_vr: Option<u32>,
    pub current_proof: Option<ProofTy>,

    pub proposal_cache: Option<ProposalCacheTy>,
    pub vr_proposal_cache: Option<ProposalCacheTy>,

    // Map over all votes this node did in the past for this block height.
    // Necessary in case it would ever need to re-request an old aggregation,
    // to not violate the protocols specifications, by sending a different contribution.
    #[beserial(len_type(u16))]
    pub own_votes: BTreeMap<(u32, Step), Option<ProposalHashTy>>,

    // Seen proposals for each round. Currently only the first received proposal is kept.
    // TODO: BtreeMap<u32, SomeCollection<(ProposalTy, Option<u32>)>,
    #[beserial(len_type(u16))]
    pub known_proposals: BTreeMap<u32, (ProposalTy, Option<u32>)>,

    // All received messages since this heights tendermint aggregations started.
    // Used to replay all messages in case a node goes down.
    // pub messages: Vec<MessageTy>

    // All results of all past aggregations, for future reference. In the state mostly for testing purposes
    // Currently unused. Will be made use of once tendermint knows of intermediate aggregation  results
    #[beserial(len_type(u16))]
    pub best_votes: BTreeMap<(u32, Step), Aggregate<ProposalHashTy, ProofTy>>,
}

impl<
        ProposalTy: ProposalTrait,
        ProposalCacheTy: ProposalCacheTrait,
        ProposalHashTy: ProposalHashTrait,
        ProofTy: ProofTrait,
    > TendermintState<ProposalTy, ProposalCacheTy, ProposalHashTy, ProofTy>
{
    pub fn new(height: u32, initial_round: u32) -> Self {
        Self {
            height,
            round: initial_round,
            step: Step::Propose,
            locked: None,
            valid: None,
            is_our_turn: None,
            current_checkpoint: Checkpoint::StartRound,
            current_proposal: None,
            current_proposal_vr: None,
            current_proof: None,
            proposal_cache: None,
            vr_proposal_cache: None,
            own_votes: BTreeMap::new(),
            known_proposals: BTreeMap::new(),
            best_votes: BTreeMap::new(),
        }
    }
}
