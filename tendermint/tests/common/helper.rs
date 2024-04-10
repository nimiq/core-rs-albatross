use std::{
    ops::Range,
    task::{Context, Poll},
    time::Duration,
};

use tokio::sync::mpsc;

use super::*;

pub fn expect_observe_proposal(
    observe_receiver: &mut mpsc::Receiver<Observe>,
) -> SignedProposalMessage<TestProposal, bool> {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    match observe_receiver.poll_recv(&mut cx) {
        Poll::Ready(Some(Observe::Proposal(proposal))) => proposal,
        _ => panic!("Should have observed a proposal"),
    }
}

pub fn expect_observe_aggregate(
    observe_receiver: &mut mpsc::Receiver<Observe>,
) -> TaggedAggregationMessage<Agg<u32>> {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    match observe_receiver.poll_recv(&mut cx) {
        Poll::Ready(Some(Observe::Aggregate(aggregate))) => aggregate,
        _ => panic!("Should have observed an aggregate"),
    }
}

pub fn expect_nothing_observed(observe_receiver: &mut mpsc::Receiver<Observe>) {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(observe_receiver.poll_recv(&mut cx).is_pending());
}

pub fn aggregate(
    validator: &Validator,
    id: (u32, Step),
    proposals: Vec<(Option<u32>, Range<usize>)>,
) {
    let map = validator
        .aggregate_senders
        .lock()
        .expect("Lock must be lockable");

    let mut c = BTreeMap::default();
    for (p, r) in proposals {
        let mut bitset = BitSet::default();
        for idx in r {
            bitset.insert(idx);
        }
        c.insert(p, bitset);
    }

    map.get(&id)
        .expect("Sender must be present in aggregation_senders")
        .try_send(Agg { contributions: c })
        .expect("try Send must not fail");
}

pub fn send_proposal(
    sender: &mut mpsc::Sender<SignedProposalMessage<TestProposal, bool>>,
    proposal: u32,
    round: u32,
    valid_round: Option<u32>,
    valid: bool,
) {
    sender
        .try_send(SignedProposalMessage {
            message: ProposalMessage {
                proposal: TestProposal(proposal),
                round,
                valid_round,
            },
            signature: valid,
        })
        .expect("try send must not fail");
}

/// Polls `tm` once, panicking for any result other than Poll::Pending.
pub fn assert_poll_pending<TProtocol: Protocol>(tm: &mut Tendermint<TProtocol>) {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(tm.poll_next_unpin(&mut cx).is_pending());
}

/// Polls `tm` once, panicking for any result other than `Poll::Ready(Some(t))`, returning `t` otherwise.
pub fn expect_poll_ready_some<TProtocol: Protocol>(
    tm: &mut Tendermint<TProtocol>,
) -> Return<TProtocol> {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);

    match tm.poll_next_unpin(&mut cx) {
        Poll::Pending => panic!("Expect_poll_ready_some must not return Poll::Pending"),
        Poll::Ready(None) => panic!("Expect_poll_ready_some must not return Poll::Ready(None)"),
        Poll::Ready(Some(r)) => r,
    }
}

pub enum Acceptance {
    Accept,
    Ignore,
    Reject,
}

pub fn expect_proposal<TProtocol: Protocol>(
    tm: &mut Tendermint<TProtocol>,
    result: Acceptance,
) -> TProtocol::Proposal {
    match (expect_poll_ready_some(tm), result) {
        (Return::ProposalAccepted(d), Acceptance::Accept) => d.message.proposal,
        (Return::ProposalIgnored(d), Acceptance::Ignore) => d.message.proposal,
        (Return::ProposalRejected(d), Acceptance::Reject) => d.message.proposal,
        _ => panic!("Poll was expected to return Return::Proposal*(_)"),
    }
}

pub fn expect_decision<TProtocol: Protocol>(tm: &mut Tendermint<TProtocol>) -> TProtocol::Decision {
    match expect_poll_ready_some(tm) {
        Return::Decision(decision) => decision,
        _ => panic!("Expected a decision"),
    }
}

pub fn expect_state<TProtocol: Protocol>(tm: &mut Tendermint<TProtocol>) -> State<TProtocol> {
    match expect_poll_ready_some(tm) {
        Return::Update(state) => state,
        _ => panic!("Expected a state"),
    }
}

pub async fn await_state<TProtocol: Protocol>(tm: &mut Tendermint<TProtocol>) -> State<TProtocol> {
    // Only wait for a timeout, otherwise this could diverge
    match nimiq_time::timeout(Duration::from_millis(1000), tm.next()).await {
        Ok(Some(Return::Update(u))) => u,
        Ok(Some(_other_returns)) => panic!("Tendermint should have returned a state."),
        Ok(None) => panic!("Tendermint should not have terminated."),
        Err(_elapsed) => panic!("Tendermint should have produced a state quicker."),
    }
}

/// Runs tendermint to completion. If at any time it returns poll pending for too long, it returns None.
/// Otherwise it runs until a decision is reached and then returns it. Only really useful for simple scenarios.
pub async fn run_tendermint(
    proposals: Vec<(
        bool,
        Option<
            SignedProposalMessage<
                <Validator as Protocol>::Proposal,
                <Validator as Protocol>::ProposalSignature,
            >,
        >,
    )>,
    prevotes: Vec<Vec<(Option<u32>, Range<usize>)>>,
    precommits: Vec<Vec<(Option<u32>, Range<usize>)>>,
    proposal_responses: Vec<(
        (u32, u32),
        SignedProposalMessage<
            <Validator as Protocol>::Proposal,
            <Validator as Protocol>::ProposalSignature,
        >,
    )>,
) -> Option<<Validator as Protocol>::Decision> {
    let mut last_state = State::<Validator>::default();

    let (observe_sender, mut receiver) = mpsc::channel::<Observe>(5);
    let mut known_proposals = BTreeMap::default();

    for (round, proposal) in proposal_responses {
        known_proposals.insert(round, proposal);
    }

    let validator = Validator {
        propose: proposals.iter().map(|t| t.0).collect(),
        observe_sender,
        known_proposals,
        aggregate_senders: Arc::new(Mutex::new(BTreeMap::default())),
    };

    let (proposal_sender, proposal_receiver) = mpsc::channel(10);
    let (_msg_sender, msg_receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        validator.clone(),
        None,
        ReceiverStream::new(proposal_receiver).boxed(),
        ReceiverStream::new(msg_receiver).boxed(),
    );

    loop {
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        // First check for outgoing communication
        match receiver.poll_recv(&mut cx) {
            Poll::Ready(Some(Observe::Aggregate(agg))) => {
                // Every time an aggregate is sent, the vote for it must have been in the previous state already.
                let vote = last_state.votes.get(&agg.tag).expect("Vote should exist");
                assert!(agg.aggregation.contributors_for(vote.as_ref()).contains(0));
            }
            Poll::Ready(Some(Observe::Proposal(p))) => {
                // If a proposal is broadcasted it must have already been in the previous states `known_proposals`
                // as well as `round_proposals`.
                // (Note a proposals hash being the proposal u32)
                assert!(last_state
                    .round_proposals
                    .get(&p.message.round)
                    .expect("Round proposals must exist")
                    .contains_key(&p.message.proposal.0));
                assert!(last_state
                    .known_proposals
                    .contains_key(&p.message.proposal.0));
            }
            Poll::Ready(None) => panic!(),
            Poll::Pending => {}
        }

        let stream_item = match tendermint.poll_next_unpin(&mut cx) {
            Poll::Pending => {
                // In case of Poll::Pending the tendermint instance is waiting for something.
                // Depending on the current state provide the necessary input as defined by the respective parameters.
                match last_state.current_step {
                    Step::Propose => {
                        let proposal = proposals.get(last_state.current_round as usize).expect("");
                        if let Some(msg) = proposal.1.clone() {
                            proposal_sender.try_send(msg).expect("");
                        }
                    }
                    Step::Prevote => {
                        let votes = prevotes
                            .get(last_state.current_round as usize)
                            .expect("")
                            .clone();
                        aggregate(&validator, (last_state.current_round, Step::Prevote), votes);
                    }
                    Step::Precommit => {
                        let votes = precommits
                            .get(last_state.current_round as usize)
                            .expect("")
                            .clone();
                        aggregate(
                            &validator,
                            (last_state.current_round, Step::Precommit),
                            votes,
                        );
                    }
                }
                tendermint.next().await
            }
            Poll::Ready(t) => t,
        };

        match stream_item {
            None => panic!(""),
            Some(Return::Decision(d)) => return Some(d),
            Some(Return::Update(state)) => {
                last_state = state;
            }
            _ => {}
        }
    }
}
