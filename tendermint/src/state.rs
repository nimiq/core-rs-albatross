use std::collections::BTreeMap;

use crate::{protocol::Protocol, utils::Step};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State<TProtocol: Protocol> {
    /// The round this tendermint instance is currently in.
    pub current_round: u32,

    /// The Step this tendermint instance is currently in.
    pub current_step: Step,

    /// Mapping of all known proposals from their respective hash
    pub known_proposals: BTreeMap<TProtocol::ProposalHash, TProtocol::Proposal>,

    /// Proposal hash for each round that is known
    pub round_proposals: BTreeMap<
        u32,
        BTreeMap<TProtocol::ProposalHash, (Option<u32>, TProtocol::ProposalSignature)>,
    >,

    /// For each round and step this tendermint instance has voted already the vote needs to be kept for future reference.
    pub votes: BTreeMap<(u32, Step), Option<TProtocol::ProposalHash>>,

    /// Best votes of all currently running aggregations
    pub best_votes: BTreeMap<(u32, Step), TProtocol::Aggregation>,

    /// caches inherents, referenced by their hash
    pub inherents: BTreeMap<TProtocol::InherentHash, TProtocol::Inherent>,

    /// If this tendermint instance is locked, keep the proposal hash around here alongside the round.
    ///
    /// Tendermint considers itself locked on a proposal whenever it is sending a precommit message for that proposal.
    /// Locking onto a value always also implies that that value is now `valid`.
    pub locked: Option<(u32, TProtocol::ProposalHash)>,

    /// If this tendermint instance has a valid round, keep the proposal hash around here alongside the round.
    ///
    /// A valid round occurs whenever Tendermint sees a valid proposal for that round at any point
    /// (even if the proposal timed out, the instance voted None and then receives and verifies the proposal) and
    /// 2f+1 votes for the proposal exists. Thus it can be different from `locked`, as the node does not need to vote for it.
    pub valid: Option<(u32, TProtocol::ProposalHash)>,
}

impl<TProtocol: Protocol> Default for State<TProtocol> {
    fn default() -> Self {
        Self {
            current_round: 0,
            current_step: Step::default(),
            known_proposals: BTreeMap::default(),
            round_proposals: BTreeMap::default(),
            votes: BTreeMap::default(),
            best_votes: BTreeMap::default(),
            inherents: BTreeMap::default(),
            locked: None,
            valid: None,
        }
    }
}
