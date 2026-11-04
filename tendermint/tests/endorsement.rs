//! A valid proposal the protocol does not endorse (see `Protocol::endorses_proposal`) gets no
//! prevote from this node on its own account, and nothing else changes: it is stored, locked on and
//! precommitted once the network forms a polka on it, re-proposed as the valid value, and prevoted
//! for when it comes back with that polka. Declining can therefore cost the proposer a round, but
//! never stall a height.

use futures::stream::{self, StreamExt};
use nimiq_tendermint::*;
use nimiq_test_log::test;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub mod common;

use self::common::{helper::*, *};

/// The proposal, and so its hash, that the validators below do not endorse.
const UNENDORSED: u32 = 3;

fn unendorsing_validator(propose: Vec<bool>) -> (Validator, mpsc::Receiver<Observe>) {
    let (mut validator, observe_receiver) = create_validator(propose, vec![]);
    validator.unendorse(UNENDORSED);
    (validator, observe_receiver)
}

/// Round 0 as every test starts it: the unendorsed proposal arrives, is accepted as valid and gets
/// a nil prevote, and the prevote aggregation starts with that nil vote.
fn round_0_nil_prevote(
    tendermint: &mut Tendermint<Validator>,
    sender: &mut mpsc::Sender<SignedProposalMessage<TestProposal, bool>>,
    observe_receiver: &mut mpsc::Receiver<Observe>,
) {
    send_proposal(sender, UNENDORSED, 0, None, true);
    expect_proposal(tendermint, Acceptance::Accept);
    let state = expect_state(tendermint);
    assert!(state.known_proposals.contains_key(&UNENDORSED));
    assert_eq!(state.current_step, Step::Prevote);
    assert_eq!(state.votes.get(&(0, Step::Prevote)), Some(&None));

    expect_nothing_observed(observe_receiver);
    let _state = expect_state(tendermint);
    let prevote = expect_observe_aggregate(observe_receiver);
    assert_eq!(prevote.tag, (0, Step::Prevote));
    assert!(prevote.aggregation.contributors_for(None).contains(0));
}

/// Round 0 with the polka on the unendorsed proposal arriving too late: this node prevotes nil,
/// times out on an inconclusive prevote aggregate and precommits nil, then sees the 2f+1 prevotes
/// and a conclusive nil precommit. It ends up in 1-Propose with the proposal as its valid value and
/// nothing locked, the state of every honest node that saw the polka too late to precommit.
async fn round_0_late_polka(
    validator: &Validator,
    tendermint: &mut Tendermint<Validator>,
    sender: &mut mpsc::Sender<SignedProposalMessage<TestProposal, bool>>,
    observe_receiver: &mut mpsc::Receiver<Observe>,
) {
    round_0_nil_prevote(tendermint, sender, observe_receiver);

    // 2f prevotes for the proposal plus this node's nil: inconclusive, so the prevote step times out.
    aggregate(
        validator,
        (0, Step::Prevote),
        vec![
            (Some(UNENDORSED), 1..Validator::TWO_F_PLUS_ONE),
            (None, 0..1),
        ],
    );
    let _state = expect_state(tendermint);
    let state = await_state(tendermint).await;
    assert_eq!(state.current_step, Step::Precommit);
    assert_eq!(state.votes.get(&(0, Step::Precommit)), Some(&None));
    assert_eq!(state.locked, None);
    expect_nothing_observed(observe_receiver);
    let _state = expect_state(tendermint);
    let precommit = expect_observe_aggregate(observe_receiver);
    assert_eq!(precommit.tag, (0, Step::Precommit));

    // The polka completes after the fact: the proposal becomes the valid value, nothing is locked.
    aggregate(
        validator,
        (0, Step::Prevote),
        vec![
            (Some(UNENDORSED), 1..Validator::TWO_F_PLUS_ONE + 1),
            (None, 0..1),
        ],
    );
    let state = expect_state(tendermint);
    assert_eq!(state.valid, Some((0, UNENDORSED)));
    assert_eq!(state.locked, None);

    // A conclusive nil precommit moves on to round 1.
    aggregate(
        validator,
        (0, Step::Precommit),
        vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
    );
    let state = expect_state(tendermint);
    assert_eq!(
        (state.current_round, state.current_step),
        (1, Step::Propose)
    );
}

/// This node prevotes nil, the rest of the network forms the polka without it, and the node then
/// locks, precommits and decides like everyone else.
#[test(tokio::test)]
async fn it_prevotes_nil_but_follows_a_polka_on_an_unendorsed_proposal() {
    let (validator, mut observe_receiver) = unendorsing_validator(vec![false]);
    let (mut sender, receiver) = mpsc::channel(10);
    let mut tendermint = Tendermint::new(
        validator.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );
    round_0_nil_prevote(&mut tendermint, &mut sender, &mut observe_receiver);

    // 2f+1 other validators prevote for the proposal.
    aggregate(
        &validator,
        (0, Step::Prevote),
        vec![(Some(UNENDORSED), 1..Validator::TWO_F_PLUS_ONE + 1)],
    );
    let state = expect_state(&mut tendermint);
    assert_eq!(state.current_step, Step::Precommit);
    assert_eq!(state.locked, Some((0, UNENDORSED)));
    assert_eq!(state.valid, Some((0, UNENDORSED)));
    assert_eq!(
        state.votes.get(&(0, Step::Precommit)),
        Some(&Some(UNENDORSED))
    );

    // The precommit aggregation starts with this node's vote for the proposal.
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    let precommit = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(precommit.tag, (0, Step::Precommit));
    assert!(precommit
        .aggregation
        .contributors_for(Some(&UNENDORSED))
        .contains(0));

    // 2f+1 precommits decide the height.
    aggregate(
        &validator,
        (0, Step::Precommit),
        vec![(Some(UNENDORSED), 0..Validator::TWO_F_PLUS_ONE)],
    );
    let decision = expect_decision(&mut tendermint);
    assert_eq!(decision.round, 0);
    assert_eq!(decision.proposal, TestProposal(UNENDORSED));
}

/// Re-proposed with its valid round by someone else, the unendorsed proposal gets this node's
/// prevote: the polka exists, so declining could only stall the height. The round then decides.
#[test(tokio::test)]
async fn it_prevotes_for_an_unendorsed_proposal_re_proposed_with_a_polka() {
    let (validator, mut observe_receiver) = unendorsing_validator(vec![false, false]);
    let (mut sender, receiver) = mpsc::channel(10);
    let mut tendermint = Tendermint::new(
        validator.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );
    round_0_late_polka(
        &validator,
        &mut tendermint,
        &mut sender,
        &mut observe_receiver,
    )
    .await;

    // Waiting for the proposal of round 1, which is the round 0 proposal with its valid round.
    assert_poll_pending(&mut tendermint);
    send_proposal(&mut sender, UNENDORSED, 1, Some(0), true);
    expect_proposal(&mut tendermint, Acceptance::Accept);
    let state = expect_state(&mut tendermint);
    assert_eq!(state.current_step, Step::Prevote);
    assert_eq!(
        state.votes.get(&(1, Step::Prevote)),
        Some(&Some(UNENDORSED))
    );

    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    let prevote = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(prevote.tag, (1, Step::Prevote));
    assert!(prevote
        .aggregation
        .contributors_for(Some(&UNENDORSED))
        .contains(0));

    // Polka, lock, precommit, decision.
    aggregate(
        &validator,
        (1, Step::Prevote),
        vec![(Some(UNENDORSED), 0..Validator::TWO_F_PLUS_ONE)],
    );
    let state = expect_state(&mut tendermint);
    assert_eq!(state.locked, Some((1, UNENDORSED)));
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    aggregate(
        &validator,
        (1, Step::Precommit),
        vec![(Some(UNENDORSED), 0..Validator::TWO_F_PLUS_ONE)],
    );
    let decision = expect_decision(&mut tendermint);
    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(UNENDORSED));
}

/// As the proposer of round 1 this node re-proposes its valid value, endorsed or not, with its
/// valid round and prevotes for it: the validators that saw the polka need the re-proposal to decide.
#[test(tokio::test)]
async fn it_re_proposes_an_unendorsed_valid_value_and_prevotes_for_it() {
    let (validator, mut observe_receiver) = unendorsing_validator(vec![false, true]);
    let (mut sender, receiver) = mpsc::channel(10);
    let mut tendermint = Tendermint::new(
        validator.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );
    round_0_late_polka(
        &validator,
        &mut tendermint,
        &mut sender,
        &mut observe_receiver,
    )
    .await;

    // The first poll sets the re-proposal, the second broadcasts it and prevotes.
    let state = expect_state(&mut tendermint);
    assert_eq!(
        state
            .round_proposals
            .get(&1)
            .and_then(|proposals| proposals.get(&UNENDORSED))
            .map(|(valid_round, _signature)| *valid_round),
        Some(Some(0)),
    );
    expect_nothing_observed(&mut observe_receiver);
    let state = expect_state(&mut tendermint);
    let proposal = expect_observe_proposal(&mut observe_receiver);
    assert_eq!(proposal.message.round, 1);
    assert_eq!(proposal.message.valid_round, Some(0));
    assert_eq!(proposal.message.proposal, TestProposal(UNENDORSED));
    assert_eq!(state.current_step, Step::Prevote);
    assert_eq!(
        state.votes.get(&(1, Step::Prevote)),
        Some(&Some(UNENDORSED))
    );
}
