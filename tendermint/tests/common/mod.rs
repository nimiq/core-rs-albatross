use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use futures::{
    future::{self, BoxFuture, FutureExt},
    stream::{BoxStream, StreamExt},
};
use nimiq_collections::BitSet;
use nimiq_tendermint::*;
use tokio::{sync::mpsc, time::timeout};
use tokio_stream::wrappers::ReceiverStream;

pub mod helper;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestProposal(pub u32);
impl Proposal<u32, u32> for TestProposal {
    fn hash(&self) -> u32 {
        self.0
    }
    fn inherent_hash(&self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TestInherent(pub u32);
impl Inherent<u32> for TestInherent {
    fn hash(&self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agg<ProposalHash> {
    pub contributions: BTreeMap<Option<ProposalHash>, BitSet>,
}

impl<ProposalHash: Send + Sync + Clone + std::fmt::Debug + Ord + 'static> Aggregation<ProposalHash>
    for Agg<ProposalHash>
{
    fn all_contributors(&self) -> BitSet {
        let mut b = BitSet::default();
        for c in self.contributions.iter() {
            b |= c.1.clone();
        }
        b
    }
    fn contributors_for(&self, vote: Option<&ProposalHash>) -> BitSet {
        self.contributions
            .get(&vote.cloned())
            .cloned()
            .unwrap_or_default()
    }
    fn proposals(&self) -> Vec<(ProposalHash, usize)> {
        self.contributions
            .iter()
            .filter_map(|(hash_opt, sig)| hash_opt.as_ref().map(|hash| (hash.clone(), sig.len())))
            .collect()
    }
}

impl<ProposalHash: Send + Sync + Clone + std::fmt::Debug + Ord + 'static>
    AggregationMessage<ProposalHash> for Agg<ProposalHash>
{
    fn sender(&self) -> u16 {
        0
    }
}

#[derive(Debug, PartialEq)]
pub struct Decision {
    pub proposal: TestProposal,
    pub inherents: TestInherent,
    pub round: u32,
    pub sig: BitSet,
}

#[derive(Debug)]
pub enum Observe {
    Proposal(SignedProposalMessage<TestProposal, bool>),
    Aggregate(TaggedAggregationMessage<Agg<u32>>),
}

#[derive(Clone, Debug)]
pub struct Validator {
    propose: Vec<bool>,
    aggregate_senders: Arc<Mutex<BTreeMap<(u32, Step), mpsc::Sender<Agg<u32>>>>>,
    /// (round, hash) => SignedProposal
    known_proposals: BTreeMap<(u32, u32), SignedProposalMessage<TestProposal, bool>>,
    observe_sender: mpsc::Sender<Observe>,
}

// Dummy PartialEq implementation such that State<Validator> implements PartialEq.
// The value of Validator does not matter, thus it always returns true.
impl PartialEq for Validator {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

pub fn create_validator(
    propose: Vec<bool>,
    proposals: Vec<((u32, u32), SignedProposalMessage<TestProposal, bool>)>,
) -> (Validator, mpsc::Receiver<Observe>) {
    let (observe_sender, receiver) = mpsc::channel::<Observe>(5);
    let mut known_proposals = BTreeMap::default();

    for (round, proposal) in proposals {
        known_proposals.insert(round, proposal);
    }

    let validator = Validator {
        propose,
        observe_sender,
        known_proposals,
        aggregate_senders: Arc::new(Mutex::new(BTreeMap::default())),
    };

    (validator, receiver)
}

impl Protocol for Validator {
    type Proposal = TestProposal;
    type ProposalHash = u32;
    type InherentHash = u32;
    type ProposalSignature = bool;
    type Inherent = TestInherent;
    type Aggregation = Agg<u32>;
    type AggregationMessage = Agg<u32>;
    type Decision = Decision;

    const F_PLUS_ONE: usize = 4;
    const TWO_F_PLUS_ONE: usize = 7;
    const TIMEOUT_INIT: u64 = 100;
    const TIMEOUT_DELTA: u64 = 100;

    fn is_proposer(&self, round: u32) -> Result<bool, ProtocolError> {
        Ok(*self.propose.get(round as usize).expect("Exceeded rounds"))
    }

    fn create_proposal(
        &self,
        round: u32,
    ) -> Result<(ProposalMessage<Self::Proposal>, Self::Inherent), ProtocolError> {
        Ok((
            ProposalMessage {
                round,
                valid_round: None,
                proposal: TestProposal(round),
            },
            TestInherent(round),
        ))
    }

    fn sign_proposal(
        &self,
        _proposal_message: &ProposalMessage<Self::Proposal>,
    ) -> Self::ProposalSignature {
        true
    }

    fn verify_proposal(
        &self,
        proposal: &SignedProposalMessage<Self::Proposal, Self::ProposalSignature>,
        precalculated_inherent: Option<Self::Inherent>,
    ) -> Result<Self::Inherent, ProposalError> {
        if proposal.signature {
            if let Some(inherent) = precalculated_inherent {
                Ok(inherent)
            } else {
                Ok(TestInherent(proposal.message.proposal.0))
            }
        } else {
            Err(ProposalError::InvalidProposal)
        }
    }

    fn verify_aggregation_message(
        &self,
        round: u32,
        step: Step,
        message: Self::AggregationMessage,
    ) -> BoxFuture<'static, Result<(), ()>> {
        let _ = (round, step, message);
        Box::pin(future::ready(Ok(())))
    }

    fn request_proposal(
        &self,
        proposal_hash: Self::ProposalHash,
        round: u32,
        _candidates: BitSet,
    ) -> BoxFuture<'static, Option<SignedProposalMessage<Self::Proposal, Self::ProposalSignature>>>
    {
        future::ready(self.known_proposals.get(&(round, proposal_hash)).cloned()).boxed()
    }

    fn create_aggregation(
        &self,
        round: u32,
        step: Step,
        vote: Option<Self::ProposalHash>,
        _update_stream: BoxStream<'static, Self::AggregationMessage>,
    ) -> BoxStream<'static, Self::Aggregation> {
        let (sender, receiver) = mpsc::channel(100);

        let mut b = BitSet::default();
        b.insert(0);
        let mut contributions = BTreeMap::default();
        contributions.insert(vote, b);
        sender
            .try_send(Agg {
                contributions: contributions.clone(),
            })
            .expect("Must be able to send initial contribution");

        self.aggregate_senders
            .lock()
            .expect("")
            .insert((round, step), sender);

        self.observe_sender
            .try_send(Observe::Aggregate(TaggedAggregationMessage {
                tag: (round, step),
                aggregation: Agg { contributions },
            }))
            .expect("Failed to send observable item");

        ReceiverStream::new(receiver).boxed()
    }

    fn create_decision(
        &self,
        proposal: Self::Proposal,
        inherents: Self::Inherent,
        aggregation: Self::Aggregation,
        round: u32,
    ) -> Self::Decision {
        let sig = aggregation
            .contributions
            .get(&Some(proposal.0))
            .expect("Proposal not present in proof")
            .clone();
        Decision {
            round,
            proposal,
            inherents,
            sig,
        }
    }

    fn broadcast_proposal(
        &self,
        proposal: SignedProposalMessage<Self::Proposal, Self::ProposalSignature>,
    ) {
        self.observe_sender
            .try_send(Observe::Proposal(proposal))
            .expect("Failed to send proposal to observer");
    }
}
