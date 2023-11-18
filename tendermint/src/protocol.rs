use futures::{future::BoxFuture, stream::BoxStream};
use nimiq_collections::BitSet;
use serde::{Deserialize, Serialize};

use crate::utils::Step;

/// Error for proposal verification. Currently not really used, but in place to allow for potential
/// punishment of misbehaving contributors to the tendermint protocol.
#[derive(Debug)]
pub enum ProposalError {
    /// Invalid proposals may be used to punish the proposer.
    InvalidProposal,
    /// Proposals with invalid signatures must be ignored, but the peer who broadcasted it may be banned.
    InvalidSignature,
    /// Collectively used for all problems not accounted for by InvalidProposal and InvalidSignature.
    Other,
}

#[derive(Clone, Debug)]
/// Unsigned Message structure containing a proposal and the corresponding round and valid round.
pub struct ProposalMessage<Proposal> {
    /// The round the proposal is proposed in.
    /// Not necessarily the same as the round the proposal was produced in.
    pub round: u32,
    /// The last round the proposal has seen 2f+1 prevotes.
    pub valid_round: Option<u32>,
    /// The actual proposal.
    pub proposal: Proposal,
}

#[derive(Clone, Debug)]
/// Signed structure for a proposal message, containing signature and proposal message.
pub struct SignedProposalMessage<Proposal, ProposalSignature> {
    pub message: ProposalMessage<Proposal>,
    pub signature: ProposalSignature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaggedAggregationMessage<AggregationMessage> {
    pub tag: (u32, Step),
    pub aggregation: AggregationMessage,
}

pub trait Aggregation<ProposalHash>:
    Send + Sync + Clone + std::fmt::Debug + Unpin + Sized + 'static
{
    /// Returns every present proposal hash (excluding None), alongside its accumulated vote count
    fn proposals(&self) -> Vec<(ProposalHash, usize)>;

    /// For a given proposal hash, or None, returns the contributors bitset
    fn contributors_for(&self, vote: Option<&ProposalHash>) -> BitSet;

    /// Returns contributors bitset for all voted on proposals and None combined.
    fn all_contributors(&self) -> BitSet;
}

/// A proposal for Tendermint. Signatures for these  proposals are two fold:
/// * the proposal itself is signed by the Validator proposing it alongside its round and valid_round.
///     The signing and verification of it is done in Deps::sign_proposal and Deps::verify_proposal.
/// * the proposal is signed, potentially using a different signature scheme for the aggregation itself.
pub trait Proposal<ProposalHash, InherentHash> {
    /// Hash of the proposal. May include parts of the inherent.
    fn hash(&self) -> ProposalHash;

    /// Identifies the inherent which goes alongside this proposal. Multiple proposals might have the same inherent.
    fn inherent_hash(&self) -> InherentHash;
}

pub trait Inherent<InherentHash> {
    /// Hash of the inherent.
    fn hash(&self) -> InherentHash;
}

#[derive(Clone, Debug)]
pub enum ProtocolError {
    Abort,
}

pub trait Protocol: Clone + Send + Sync + Unpin + Sized + 'static {
    type Proposal: Proposal<Self::ProposalHash, Self::InherentHash>
        + Unpin
        + Clone
        + std::fmt::Debug;
    type ProposalHash: Unpin + Clone + Send + Sync + std::fmt::Debug + Ord;
    type InherentHash: Unpin + Clone + std::fmt::Debug + Ord;
    type ProposalSignature: Clone + Unpin;
    type Inherent: Inherent<Self::InherentHash> + Unpin + Clone + std::fmt::Debug;
    type AggregationMessage: Aggregation<Self::ProposalHash> + Send + Unpin;
    type Aggregation: Aggregation<Self::ProposalHash> + Unpin;
    type Decision: Unpin;

    const TIMEOUT_DELTA: u64;
    const TIMEOUT_INIT: u64;
    const TWO_F_PLUS_ONE: usize;
    const F_PLUS_ONE: usize;

    /// Returns whether or not the validator this node is configured for is the block producer for given `round`
    fn is_proposer(&self, round: u32) -> Result<bool, ProtocolError>;

    /// Creates the proposal for given `round`
    fn create_proposal(
        &self,
        round: u32,
    ) -> Result<(ProposalMessage<Self::Proposal>, Self::Inherent), ProtocolError>;

    /// Signs a given `proposal_message` for sending it over the wire
    fn sign_proposal(
        &self,
        proposal_message: &ProposalMessage<Self::Proposal>,
    ) -> Self::ProposalSignature;

    /// Verifies a given `proposal`. Optionally a precomputed `precalculated_inherent` can be provided if the inherent has been computed before.
    /// All checks except for the signature verification can be skipped using the `signature_only` flag
    fn verify_proposal(
        &self,
        proposal: &SignedProposalMessage<Self::Proposal, Self::ProposalSignature>,
        precalculated_inherent: Option<Self::Inherent>,
        signature_only: bool,
    ) -> Result<Self::Inherent, ProposalError>;

    /// Broadcasts a signed proposal given as `proposal`.
    /// Returns nothing, as there is no way of reacting to not being able to broadcast a proposal.
    fn broadcast_proposal(
        &self,
        proposal: SignedProposalMessage<Self::Proposal, Self::ProposalSignature>,
    );

    /// Requests for given `round` a proposal with hash `proposal_hash` from `candidates`.
    ///
    /// A reasonable effort should be made to ask the candidates, but the future might return None if there is no appropriate answer.
    fn request_proposal(
        &self,
        proposal_hash: Self::ProposalHash,
        round: u32,
        candidates: BitSet,
    ) -> BoxFuture<'static, Option<SignedProposalMessage<Self::Proposal, Self::ProposalSignature>>>;

    /// Creates the decision for `round` out of the given parts.
    fn create_decision(
        &self,
        proposal: Self::Proposal,
        inherent: Self::Inherent,
        aggregation: Self::Aggregation,
        round: u32,
    ) -> Self::Decision;

    /// Creates an Aggregation `impl Stream<Item = Self::Aggregation>` for `round`, `step`.
    /// It will use `vote` to create this nodes contribution to the aggregation.
    fn create_aggregation(
        &self,
        round: u32,
        step: Step,
        vote: Option<Self::ProposalHash>,
        update_stream: BoxStream<'static, Self::AggregationMessage>,
    ) -> BoxStream<'static, Self::Aggregation>;
}
