use std::collections::BTreeMap;

use futures::stream::{self, StreamExt};
use nimiq_collections::BitSet;
use nimiq_tendermint::*;
use nimiq_test_log::test;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub mod common;

use self::common::{helper::*, *};

#[tokio::test]
async fn it_exports_state_before_broadcast() {
    let (validator, mut observe_receiver) = create_validator(vec![true], vec![]);

    let mut tendermint = Tendermint::new(
        validator,
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // The node is the producer, so a state export is expected after the proposal was produced.
    let state = expect_state(&mut tendermint);
    // The step must still be proposed
    assert_eq!(state.current_step, Step::Propose);
    // get the proposal in the state
    let proposal = state
        .known_proposals
        .get(&0)
        .expect("The proposal should exist.");

    // The proposal must not have been broadcast yet.
    expect_nothing_observed(&mut observe_receiver);
    // Polling again must broadcast the proposal.
    let _state = expect_state(&mut tendermint);
    let observed_proposal = expect_observe_proposal(&mut observe_receiver);
    assert!(observed_proposal.signature);
    assert_eq!(observed_proposal.message.round, 0);
    assert_eq!(observed_proposal.message.valid_round, None);
    assert_eq!(proposal, &observed_proposal.message.proposal);
}

#[tokio::test]
async fn it_can_handle_proposal_basics() {
    // As this test will only test the very first proposal, just create a validator not proposing in round 0.
    let (proposer, mut observe_receiver) = create_validator(vec![false], vec![]);

    let (mut proposal_sender, proposal_receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(proposal_receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Poll the tendermint instance, it should not do anything and return poll pending as there is no proposal.
    // This would start the timeout.
    assert_poll_pending(&mut tendermint);

    // Send a proposal with invalid signature
    send_proposal(&mut proposal_sender, 0, 0, None, false);
    // Make sure it gets rejected.
    expect_proposal(&mut tendermint, Acceptance::Reject);

    // Do not send a new proposal for now. This should lead to a timeout in tendermint, resulting in a None vote.
    // The timeout for returning a state is larger than the internal timeout for the proposal for round 0.
    let state = await_state(&mut tendermint).await;
    // No known proposals
    assert_eq!(state.known_proposals.len(), 0);
    // state is 0-Prevote
    assert_eq!(state.current_round, 0);
    assert_eq!(state.current_step, Step::Prevote);
    // vote is for None
    assert_eq!(
        state
            .votes
            .get(&(0, Step::Prevote))
            .expect("Vote for 0-Prevote must exist"),
        &None,
    );

    // Send a valid proposal for 0. This proposal must be accepted, as there has not been one yet and it is valid.
    send_proposal(&mut proposal_sender, 9, 0, None, true);
    // Make sure it gets accepted
    expect_proposal(&mut tendermint, Acceptance::Accept);

    // Send another valid proposal for 0. This proposal must be ignored, as there has already been one.
    send_proposal(&mut proposal_sender, 5, 0, None, true);
    // make sure it gets ignored.
    expect_proposal(&mut tendermint, Acceptance::Ignore);

    // Send another invalid proposal for 0. This proposal must be ignored, as there has already been one.
    // Note that it is not rejected.
    send_proposal(&mut proposal_sender, 5, 0, None, false);
    // make sure it gets ignored.
    expect_proposal(&mut tendermint, Acceptance::Ignore);

    // Make sure no aggregation message was sent. Meaning the aggregation has not started yet.
    expect_nothing_observed(&mut observe_receiver);
    // A state should be pending. Also only at this point the aggregate shall be started.
    // So this is the first time an aggregate message can be observed.
    let state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    // The state must contain a single proposal for round 0: 9
    let proposals = state
        .round_proposals
        .get(&0)
        .expect("There should be a proposals set for round 0");
    assert_eq!(proposals.len(), 1);
    assert_eq!(
        proposals
            .iter()
            .next()
            .expect("Set with len == 1 must return an item for first()"),
        (&9u32, &(None, true))
    );
    // the state must also still contain the vote for None
    assert_eq!(
        state
            .votes
            .get(&(0, Step::Prevote))
            .expect("Vote for 0-Prevote must exist"),
        &None,
    );
}

#[tokio::test]
async fn it_returns_pending_state() {
    // As this test will only test the very first proposal, just create a validator not proposing in round 0.
    let (proposer, mut observe_receiver) = create_validator(vec![false], vec![]);

    let (mut proposal_sender, proposal_receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer,
        None,
        ReceiverStream::new(proposal_receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Send valid proposal.
    send_proposal(&mut proposal_sender, 0, 0, None, true);
    // Expect the proposal to be returned next as accepted
    expect_proposal(&mut tendermint, Acceptance::Accept);

    // Expect a state update which should be pending from the previously received proposal.
    let state = expect_state(&mut tendermint);
    // The aggregate message must not have been sent yet as the vote needs to be persisted first. (which this state should do)
    expect_nothing_observed(&mut observe_receiver);
    // Make sure the vote is contained in the state
    assert!(state.votes.contains_key(&(0, Step::Prevote)));
    assert_eq!(state.known_proposals.len(), 1);
}

#[tokio::test]
async fn it_returns_pending_state_for_delayed_proposal() {
    // As this test will only test the very first proposal, just create a validator not proposing in round 0.
    let (proposer, mut observe_receiver) = create_validator(vec![false], vec![]);

    let (mut proposal_sender, proposal_receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(proposal_receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Wait for the proposal to timeout.
    let state = await_state(&mut tendermint).await;
    assert!(state.known_proposals.is_empty());
    assert!(state.votes.contains_key(&(0, Step::Prevote)));

    // Send valid proposal.
    send_proposal(&mut proposal_sender, 0, 0, None, true);
    // Expect the proposal to be returned next as accepted
    expect_proposal(&mut tendermint, Acceptance::Accept);
    // Proposal must not be broadcasted.
    expect_nothing_observed(&mut observe_receiver);
    // Expect a state update which should be pending from the previously received proposal.
    let state = expect_state(&mut tendermint);
    assert_eq!(state.known_proposals.len(), 1);
}

#[tokio::test]
async fn it_returns_pending_state_for_past_round_proposal() {
    // This test will test receiving the proposal for round 0 while already progressing through round 1.
    // Let the validator propose in round 1 but not round 0.
    let (proposer, mut observe_receiver) = create_validator(vec![false, true], vec![]);

    let (mut proposal_sender, proposal_receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(proposal_receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Wait for the proposal to timeout. This will create the vote, but it will not create the aggregation as that would send the vote.
    let state = await_state(&mut tendermint).await;
    assert_eq!(
        state
            .votes
            .get(&(0, Step::Prevote))
            .expect("Vote must exist"),
        &None
    );
    assert!(state.best_votes.get(&(0, Step::Prevote)).is_none());

    expect_nothing_observed(&mut observe_receiver);

    // Next up should be another state with the aggregation started and best_votes containing a value for 0-prevote
    let state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    assert!(state
        .best_votes
        .get(&(0, Step::Prevote))
        .expect("best vote must exist")
        .contributions
        .get(&None)
        .expect("Votes for None must exist")
        .contains(0),);

    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
    );

    // Next state should contain the conclusive 0-Prevote aggregate and a vote against all proposals for 0-Precommit
    let state = expect_state(&mut tendermint);
    // No vote must have been sent, as the state containing the vote has only just been returned.
    expect_nothing_observed(&mut observe_receiver);
    assert_eq!(
        state
            .best_votes
            .get(&(0, Step::Prevote))
            .expect("best vote must exist")
            .contributions
            .get(&None)
            .expect("Votes for None must exist")
            .len(),
        Validator::TWO_F_PLUS_ONE,
    );
    assert_eq!(
        state
            .votes
            .get(&(0, Step::Precommit))
            .expect("Vote must exist"),
        &None
    );
    assert!(state.best_votes.get(&(0, Step::Precommit)).is_none());

    // Next up should be another state with the aggregation started and best_votes containing a value for 0-precommit
    let state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    assert!(state
        .best_votes
        .get(&(0, Step::Precommit))
        .expect("best vote must exist")
        .contributions
        .get(&None)
        .expect("Votes for None must exist")
        .contains(0),);

    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
    );

    // Next state should be a state where the None vote for 0-precommit got 2f+1 votes,
    // thus advancing to 1-propose.
    let state = expect_state(&mut tendermint);
    assert_eq!(
        state
            .best_votes
            .get(&(0, Step::Precommit))
            .expect("best vote must exist")
            .contributions
            .get(&None)
            .expect("Votes for None must exist")
            .len(),
        Validator::TWO_F_PLUS_ONE,
    );
    assert_eq!(state.current_round, 1);
    assert_eq!(state.current_step, Step::Propose);
    assert_eq!(state.known_proposals.len(), 0);

    // Proposal is created but not yet sent. No vote exists yet.
    let state = expect_state(&mut tendermint);
    // Make sure nothing was observed since the last aggregate.
    expect_nothing_observed(&mut observe_receiver);
    assert_eq!(state.known_proposals.len(), 1);
    assert!(state.votes.get(&(1, Step::Prevote)).is_none());

    // Proposal is sent and the vote is created.
    let state = expect_state(&mut tendermint);
    expect_observe_proposal(&mut observe_receiver);
    assert_eq!(
        state
            .votes
            .get(&(1, Step::Prevote))
            .expect("Vote must exist"),
        &Some(1)
    );
    assert!(state.best_votes.get(&(1, Step::Prevote)).is_none());

    // aggregation is created
    let state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    assert!(state
        .best_votes
        .get(&(1, Step::Prevote))
        .expect("best vote must exist")
        .contributions
        .get(&Some(1))
        .expect("Votes for None must exist")
        .contains(0),);

    // Send incomplete aggregation while also sending the proposal for round 0.
    aggregate(
        &proposer,
        (1, Step::Prevote),
        vec![(Some(1), 0..Validator::F_PLUS_ONE)],
    );
    send_proposal(&mut proposal_sender, 0, 0, None, true);

    // expect the proposal to be returned next as accepted
    expect_proposal(&mut tendermint, Acceptance::Accept);
    // expect a state update which should be pending from the received proposal.
    let state = expect_state(&mut tendermint);
    assert_eq!(state.known_proposals.len(), 2);
    assert_eq!(
        state
            .best_votes
            .get(&(1, Step::Prevote))
            .expect("best vote must exist")
            .contributions
            .get(&Some(1))
            .expect("Votes for None must exist")
            .len(),
        Validator::F_PLUS_ONE,
    );

    // Lastly, there should not be another state update here.
    assert_poll_pending(&mut tendermint);
    expect_nothing_observed(&mut observe_receiver);
}

#[tokio::test]
async fn it_requests_missing_proposals() {
    // As this test will only test the very first proposal, just create a validator not proposing in round 0.
    let (proposer, mut observe_receiver) = create_validator(
        vec![false],
        vec![(
            // Know proposal for round 0 with hash 0 such that it can be requested
            (0, 0),
            SignedProposalMessage {
                signature: true,
                message: ProposalMessage {
                    round: 0,
                    valid_round: None,
                    proposal: TestProposal(0),
                },
            },
        )],
    );

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Wait for the proposal to timeout.
    let state = await_state(&mut tendermint).await;
    assert!(state.known_proposals.is_empty());
    expect_nothing_observed(&mut observe_receiver);

    // Next state is started 0-prevote aggregation with a none vote.
    let state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    assert!(state.known_proposals.is_empty());

    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(None, 0..1), (Some(0), 1..(Validator::F_PLUS_ONE + 1))],
    );

    // expect a state update which should be the new improved aggregation
    let state = expect_state(&mut tendermint);
    assert!(state.known_proposals.is_empty());
    let best = state
        .best_votes
        .get(&(0, Step::Prevote))
        .expect("best vote must exist");
    assert_eq!(
        best.contributions
            .get(&None)
            .expect("Must contain None vote.")
            .len(),
        1
    );
    assert!(
        best.contributions
            .get(&Some(0))
            .expect("Must contain Some(0) vote")
            .len()
            >= Validator::F_PLUS_ONE
    );

    let state = expect_state(&mut tendermint);
    expect_nothing_observed(&mut observe_receiver);
    assert_eq!(state.known_proposals.len(), 1);
}

/// * proposes, votes Block
/// * sees full 0-prevote and locks vote Block
/// * sees empty 0-precommit and proceeds to round 1
/// * proposal times out votes None
/// * sees full 1-prevote for unknown proposal, votes None and keeps being locked and keeps having (0, 0) as valid value
#[tokio::test]
async fn it_does_not_unlock() {
    let (proposer, mut observe_receiver) = create_validator(vec![true, false], vec![]);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Wait for the proposal to be produced to get the first state
    let update = expect_state(&mut tendermint);
    // The proposal must not be sent, before the state has been exported at least once.
    expect_nothing_observed(&mut observe_receiver);
    assert_eq!(update.known_proposals.len(), 1);
    assert!(update.round_proposals.contains_key(&0u32));
    assert!(update.round_proposals.contains_key(&0u32));

    // The next update must include the vote for the proposal as the proposal has now been broadcasted.
    let update = expect_state(&mut tendermint);
    // Make sure the proposal was observed.
    let _proposal = expect_observe_proposal(&mut observe_receiver);
    // Nothing else must be observed.
    expect_nothing_observed(&mut observe_receiver);
    // Make sure the vote exists.
    assert!(update.votes.contains_key(&(0u32, Step::Prevote)));

    // In the next state a started aggregation for 0.prevote is expected.
    // It should have this nodes vote as well, voting for the proposal Some(0).
    let update = expect_state(&mut tendermint);
    let _aggregate = expect_observe_aggregate(&mut observe_receiver);
    assert!(update
        .best_votes
        .get(&(0u32, Step::Prevote))
        .expect("An aggregation 0.prevote should exist")
        .contributions
        .get(&Some(0))
        .expect("A vote for proposal Some(0) should exist")
        .contains(0),);

    // Send full prevote aggregation for the proposal Some(0).
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // Wait for the next update, which should include the vote for 0.precommit.
    // It should also include set locked and valid values.
    let update = expect_state(&mut tendermint);
    assert!(update.votes.contains_key(&(0u32, Step::Precommit)));
    assert!(update.locked.is_some());
    assert!(update.valid.is_some());
    expect_nothing_observed(&mut observe_receiver);

    // The next state should present a started 0.precommit aggregation, with this nodes contribution to Some(0)
    // as its best
    let update = expect_state(&mut tendermint);
    let _aggregate = expect_observe_aggregate(&mut observe_receiver);
    assert!(update
        .best_votes
        .get(&(0u32, Step::Precommit))
        .expect("An aggregation 0.Precommit should exist")
        .contributions
        .get(&Some(0))
        .expect("A vote for proposal Some(0) should exist")
        .contains(0),);

    // send prevote aggregation for everyone except 1 (which is this node) voting against the block
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(None, 1..(Validator::TWO_F_PLUS_ONE + 1)), (Some(0), 0..1)],
    );

    // Wait for the next state. It must have progressed to round 1.propose as no decision must have been reached.
    let update = expect_state(&mut tendermint);
    assert_eq!(update.current_round, 1);
    assert_eq!(update.current_step, Step::Propose);

    // Waiting for the next state lets the proposal time out. Thus the state should include the vote for 1.prevote
    let update = await_state(&mut tendermint).await;
    assert_eq!(update.current_step, Step::Prevote);
    assert!(update.votes.contains_key(&(1, Step::Prevote)));

    expect_nothing_observed(&mut observe_receiver);
    // The next state should present a started 1.prevote aggregation, with this nodes contribution to None
    // as its best.
    let update = expect_state(&mut tendermint);
    let _aggregate = expect_observe_aggregate(&mut observe_receiver);
    assert!(update
        .best_votes
        .get(&(1u32, Step::Prevote))
        .expect("An aggregation 1.prevote should exist")
        .contributions
        .get(&None)
        .expect("A vote against all proposals should exist")
        .contains(0),);

    // Send full prevote aggregation for unknown proposal 1.
    // Important to note here is that proposal 1 cannot successfully be requested.
    aggregate(
        &proposer,
        (1, Step::Prevote),
        vec![(Some(1), 1..(Validator::TWO_F_PLUS_ONE + 1)), (None, 0..1)],
    );

    // See the prevote, request Some(1), start the timeout as this aggregate might improve.
    let update = expect_state(&mut tendermint);
    assert_eq!(update.known_proposals.len(), 1);

    // After waiting for the next state the node must have recognized 1.prevote after the timeout elapses.
    // Since it is locked and does not know the proposal `1` it cannot unlock itself.
    // Thus the state must contain the vote against all proposals. `valid` should also remain at (0, 0)
    let update = await_state(&mut tendermint).await;
    assert_eq!(update.votes.get(&(1, Step::Precommit)), Some(&None));
    assert_eq!(update.valid, Some((0, 0)));
}

// Simple test where everything goes normally. node produces a proposal and then it gets signed in both steps.
#[test(tokio::test)]
async fn it_works_as_proposer() {
    let (proposer, mut observe_receiver) = create_validator(vec![true], vec![]);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // create proposal. This will must not broadcast it, thus also not advancing step
    let update = await_state(&mut tendermint).await;
    assert!(update.known_proposals.contains_key(&0));
    assert!(update.round_proposals.contains_key(&0));
    assert_eq!(update.current_step, Step::Propose);

    expect_nothing_observed(&mut observe_receiver);
    // This will send the proposal. Step must be Prevote now and vote must exist.
    let update = await_state(&mut tendermint).await;
    let _proposal = expect_observe_proposal(&mut observe_receiver);
    assert!(update.votes.contains_key(&(0u32, Step::Prevote)));
    assert_eq!(update.current_step, Step::Prevote);

    // This should have the single contribution of this node
    let update = await_state(&mut tendermint).await;
    let _aggregate = expect_observe_aggregate(&mut observe_receiver);
    assert!(update.best_votes.contains_key(&(0, Step::Prevote)));

    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // witness full prevote
    let update = await_state(&mut tendermint).await;
    assert!(update.votes.contains_key(&(0u32, Step::Precommit)));

    // create aggregation with no pending agg for 0-precommit
    expect_nothing_observed(&mut observe_receiver);
    let update = await_state(&mut tendermint).await;
    let _aggregate = expect_observe_aggregate(&mut observe_receiver);

    assert!(update.best_votes.contains_key(&(0u32, Step::Precommit)));

    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // witness full precommit, produce decision
    let decision = expect_decision(&mut tendermint);
    assert_eq!(decision.proposal.0, 0);
    assert_eq!(decision.inherents.0, 0);
    assert_eq!(decision.round, 0);
    assert_eq!(decision.sig.len(), Validator::TWO_F_PLUS_ONE);
}

// Simple test where everything goes normally. node produces a proposal and then it gets signed in both steps.
#[test(tokio::test)]
async fn it_works_as_non_proposer() {
    let (proposer, mut observe_receiver) = create_validator(vec![false], vec![]);

    let (mut proposal_sender, proposal_receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(proposal_receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // No proposal sent yet. Should be pending but also starting the timeout for the proposal.
    assert_poll_pending(&mut tendermint);

    // Send the proposal for round 0
    send_proposal(&mut proposal_sender, 0, 0, None, true);
    // Make sure Tendermint receives it.
    expect_proposal(&mut tendermint, Acceptance::Accept);

    // A state should be pending here, containing the vote (but not a best_vote) for 0-prevote
    // as well as the proposal
    let update = expect_state(&mut tendermint);
    assert!(update.known_proposals.contains_key(&0));
    assert!(update.round_proposals.contains_key(&0));
    assert!(update.votes.contains_key(&(0, Step::Prevote)));
    assert!(!update.best_votes.contains_key(&(0, Step::Prevote)));
    expect_nothing_observed(&mut observe_receiver);

    // After having received the proposal and setting up the vote the aggregate should have started.
    let update = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(update.current_step, Step::Prevote);
    assert!(update.best_votes.contains_key(&(0, Step::Prevote)));

    // Send sufficient 0-prevote aggregation.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // Receive the aggregate. Create a vote for 0-precommit, but do not send it
    // (no observing a vote being send, no best vote present in the update).
    let update = expect_state(&mut tendermint);
    assert!(update.votes.contains_key(&(0, Step::Precommit)));
    assert!(!update.best_votes.contains_key(&(0, Step::Precommit)));
    assert_eq!(update.current_step, Step::Precommit);
    expect_nothing_observed(&mut observe_receiver);

    // This should have the single contribution of this node
    let update = expect_state(&mut tendermint);
    assert!(update.best_votes.contains_key(&(0, Step::Precommit)));
    let _aggregate = expect_observe_aggregate(&mut observe_receiver);

    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // Witness full precommit, produce decision
    let decision = expect_decision(&mut tendermint);
    assert_eq!(decision.proposal.0, 0);
    assert_eq!(decision.inherents.0, 0);
    assert_eq!(decision.round, 0);
    assert_eq!(decision.sig.len(), Validator::TWO_F_PLUS_ONE);
}

// The first proposer does not send a proposal. The validator progresses to round 1 regardless.
#[test(tokio::test)]
async fn it_progresses_with_no_proposal() {
    let (proposer, mut observe_receiver) = create_validator(vec![false], vec![]);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // wait for a timeout for the first proposal
    let update = await_state(&mut tendermint).await;
    assert_eq!(update.votes.get(&(0u32, Step::Prevote)), Some(&None));
    assert!(!update.best_votes.contains_key(&(0u32, Step::Prevote)));

    expect_nothing_observed(&mut observe_receiver);
    // create aggregation with no pending agg for 0-prevote
    let update = expect_state(&mut tendermint);
    assert_eq!(
        update
            .best_votes
            .get(&(0u32, Step::Prevote))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        1,
    );
    expect_observe_aggregate(&mut observe_receiver);

    // vote for unknown proposal
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(99), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See the conclusive 0-prevote.
    let update = expect_state(&mut tendermint);
    assert_eq!(
        update
            .best_votes
            .get(&(0u32, Step::Prevote))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        Validator::TWO_F_PLUS_ONE,
    );

    // As this node does not know the 99 proposal it will wait for the duration of the timeout to receive it before voting None.
    let update = await_state(&mut tendermint).await;
    assert_eq!(update.votes.get(&(0u32, Step::Precommit)), Some(&None));
    assert!(!update.best_votes.contains_key(&(0u32, Step::Precommit)));

    expect_nothing_observed(&mut observe_receiver);
    // create aggregation with no pending agg for 0-prevote
    let update = expect_state(&mut tendermint);
    assert_eq!(
        update
            .best_votes
            .get(&(0u32, Step::Precommit))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        1,
    );
    expect_observe_aggregate(&mut observe_receiver);

    // vote for unknown proposal
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(Some(99), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See the sufficient 0-precommit
    let update = expect_state(&mut tendermint);
    assert_eq!(
        update
            .best_votes
            .get(&(0u32, Step::Precommit))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        Validator::TWO_F_PLUS_ONE,
    );

    // As this node does not know the 99 proposal it will wait for the duration of the timeout to receive it
    // before progressing to the next round.
    let update = await_state(&mut tendermint).await;
    assert_eq!(update.current_step, Step::Propose);
    assert_eq!(update.current_round, 1);
}

// The first proposer does not send a proposal but the second one does.
#[test(tokio::test)]
async fn it_skips_ahead() {
    let (proposer, mut observe_receiver) = create_validator(vec![false, false, false], vec![]);
    let (sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        ReceiverStream::new(receiver).boxed(),
    );

    // create Aggregation Message for round 1 with >= f+1 vote power
    let mut bs = BitSet::default();
    for index in 0..Validator::F_PLUS_ONE {
        bs.insert(index);
    }

    let mut contributions = BTreeMap::default();
    contributions.insert(None, bs);

    let am = Agg { contributions };

    sender
        .try_send(TaggedAggregationMessage {
            tag: (2, Step::Prevote),
            aggregation: am,
        })
        .expect("try_send must not fail");

    let update = await_state(&mut tendermint).await;
    expect_nothing_observed(&mut observe_receiver);
    assert_eq!(update.current_round, 2);
}

// // We don't have enough prevotes for the first proposal we create.
#[test(tokio::test)]
async fn not_enough_prevotes() {
    let (proposer, mut observe_receiver) = create_validator(vec![true, true], vec![]);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Create proposal
    let update = expect_state(&mut tendermint);
    assert!(update.known_proposals.contains_key(&0));

    // Broadcast proposal
    expect_nothing_observed(&mut observe_receiver);
    let update = expect_state(&mut tendermint);
    assert_eq!(update.votes.get(&(0, Step::Prevote)), Some(&Some(0)));
    assert!(!update.best_votes.contains_key(&(0, Step::Prevote)));
    expect_observe_proposal(&mut observe_receiver);

    // Start Aggregation
    let update = expect_state(&mut tendermint);
    assert_eq!(update.votes.get(&(0, Step::Prevote)), Some(&Some(0)));
    assert_eq!(
        update
            .best_votes
            .get(&(0, Step::Prevote))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        1,
    );
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with f+1 for 2 proposals, one of which is known.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![
            (Some(0), 0..Validator::F_PLUS_ONE),
            (Some(1), Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
        ],
    );

    // This must be conclusive (thus not awaiting a state but expecting one).
    // The node must vote None, as neither of the 2 proposals can reach 2f+1.
    let update = expect_state(&mut tendermint);
    assert_eq!(update.votes.get(&(0, Step::Precommit)), Some(&None));
}

// We don't have enough precommits for the first proposal we received.
#[test(tokio::test)]
async fn not_enough_precommits() {
    let (proposer, mut observe_receiver) = create_validator(vec![true, true], vec![]);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        stream::iter(vec![]).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Create proposal
    let update = expect_state(&mut tendermint);
    assert!(update.known_proposals.contains_key(&0));

    // Broadcast proposal
    expect_nothing_observed(&mut observe_receiver);
    let update = expect_state(&mut tendermint);
    assert_eq!(update.votes.get(&(0, Step::Prevote)), Some(&Some(0)));
    assert!(!update.best_votes.contains_key(&(0, Step::Prevote)));
    expect_observe_proposal(&mut observe_receiver);

    // Start Aggregation
    let update = expect_state(&mut tendermint);
    assert_eq!(
        update
            .best_votes
            .get(&(0, Step::Prevote))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        1,
    );
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // progress to precommit with the 0-prevote result, voting for 0
    let update = expect_state(&mut tendermint);
    assert_eq!(update.votes.get(&(0, Step::Precommit)), Some(&Some(0)));
    assert!(!update.best_votes.contains_key(&(0, Step::Precommit)));

    // Start Aggregation
    let update = expect_state(&mut tendermint);
    assert_eq!(
        update
            .best_votes
            .get(&(0, Step::Precommit))
            .expect("best_vote must exist")
            .all_contributors()
            .len(),
        1,
    );
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with f+1 for 2 proposals, one of which is known.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![
            (Some(0), 0..Validator::F_PLUS_ONE),
            (Some(1), Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
        ],
    );

    // This must be conclusive (thus not awaiting a state but expecting one).
    // The node must vote None, as neither of the 2 proposals can reach 2f+1.
    let update = expect_state(&mut tendermint);
    assert_eq!(update.current_step, Step::Propose);
    assert_eq!(update.current_round, 1);
}

// // Our validator locks on the first round proposal, which doesn't complete, and then needs to
// // rebroadcast the proposal since he is the next proposer.
#[test(tokio::test)]
async fn it_locks_and_rebroadcasts() {
    // Validator shall propose in round 1 but not 0
    let (proposer, mut observe_receiver) = create_validator(vec![false, true], vec![]);
    let (mut sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Tendermint waits for a proposal
    assert_poll_pending(&mut tendermint);

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 0, None, true);
    let original_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);
    expect_nothing_observed(&mut observe_receiver);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote
    let _state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // Start next aggregation.
    expect_nothing_observed(&mut observe_receiver);
    let state = expect_state(&mut tendermint);
    assert_eq!(state.locked, Some((0, 0)));
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with f+1 for 2 proposals, one of which is known.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![
            (Some(0), 0..Validator::F_PLUS_ONE),
            (Some(1), Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
        ],
    );

    // See the vote and progress to 1-Propose
    let state = expect_state(&mut tendermint);
    assert_eq!(state.current_round, 1);
    assert_eq!(state.current_step, Step::Propose);

    // Propose for round 1
    let state = expect_state(&mut tendermint);
    assert!(!state
        .round_proposals
        .get(&1)
        .expect("Set of proposals for round 1 must exist")
        .is_empty());

    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    // Make sure the correct proposal can be observed as being broadcast here.
    let proposal = expect_observe_proposal(&mut observe_receiver);
    assert_eq!(proposal.message.round, 1);
    assert_eq!(proposal.message.valid_round, Some(0));
    assert_eq!(proposal.message.proposal, original_proposal);

    let _state = expect_state(&mut tendermint);
    // Make sure the node correctly votes for the value it is locked on.
    let aggregate = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(aggregate.tag, (1, Step::Prevote));
    assert_eq!(aggregate.aggregation.all_contributors().len(), 1);
    assert!(aggregate.aggregation.contributors_for(Some(&0)).contains(0));
}

// Our validator locks on the first round proposal, which doesn't complete, and then needs to unlock
// on the second round when that proposal gets 2f+1 prevotes.
#[test(tokio::test)]
async fn it_locks_and_unlocks() {
    // Validator shall propose in round 2
    let (proposer, mut observe_receiver) = create_validator(vec![false, false, true], vec![]);
    let (mut sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 0, None, true);
    // Proposal must be accepted
    let _original_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // Start 0-precommit aggregation.
    expect_nothing_observed(&mut observe_receiver);
    let state = expect_state(&mut tendermint);
    // make sure the validator locks on (0, 0)
    assert_eq!(state.locked, Some((0, 0)));
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with f+1 for 2 proposals, one of which is known.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![
            (Some(0), 0..Validator::F_PLUS_ONE),
            (Some(1), Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
        ],
    );

    // See the vote and progress to 1-Propose
    let state = expect_state(&mut tendermint);
    assert_eq!(state.current_round, 1);
    assert_eq!(state.current_step, Step::Propose);

    // Validator waits for proposal
    assert_poll_pending(&mut tendermint);

    // Send good proposal for round 1
    send_proposal(&mut sender, 1, 1, None, true);
    // Proposal must be accepted
    let second_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote, validator must vote None as it is locked on a different value.
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    let a = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(a.tag, (1, Step::Prevote));
    assert_eq!(a.aggregation.all_contributors().len(), 1);
    assert!(a.aggregation.contributors_for(None).contains(0));

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (1, Step::Prevote),
        vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // Start next aggregation.
    // Validator may now vote for the proposal, as it has gathered 2f+1 prevotes itself.
    expect_nothing_observed(&mut observe_receiver);
    let state = expect_state(&mut tendermint);
    assert_eq!(state.locked, Some((1, 1)));
    let a = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(a.tag, (1, Step::Precommit));
    assert_eq!(a.aggregation.all_contributors().len(), 1);
    assert!(a
        .aggregation
        .contributors_for(Some(&second_proposal.0))
        .contains(0));
}

// The validator sees an incomplete vote for 0 prevote.
// At a later time it receives a better aggregate.
#[test(tokio::test)]
async fn it_updates_best_votes_retroactively() {
    // Validator shall not propose.
    let (proposer, mut observe_receiver) = create_validator(vec![false], vec![]);
    let (mut sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 0, None, true);
    // Proposal must be accepted
    let _original_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![
            (Some(0), 0..Validator::TWO_F_PLUS_ONE - 1),
            (
                None,
                Validator::TWO_F_PLUS_ONE..Validator::TWO_F_PLUS_ONE + 1,
            ),
        ],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // wait for the improvement timeout to expire.
    let state = await_state(&mut tendermint).await;

    // Start 0-precommit aggregation. As the vote could improve an await is necessary here, to wait the timeout.
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    expect_observe_aggregate(&mut observe_receiver);
    // Make sure the best_vote is the one given above.
    let best_vote = state
        .best_votes
        .get(&(0, Step::Prevote))
        .expect("best vote for 0-prevote must exist");
    assert_eq!(
        best_vote.all_contributors().len(),
        Validator::TWO_F_PLUS_ONE
    );

    // Vote with 2f+2 for the proposal. An additional vote for the previous aggregate.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![
            (Some(0), 0..Validator::TWO_F_PLUS_ONE),
            (
                None,
                Validator::TWO_F_PLUS_ONE..Validator::TWO_F_PLUS_ONE + 1,
            ),
        ],
    );

    let state = expect_state(&mut tendermint);
    let best_vote = state
        .best_votes
        .get(&(0, Step::Prevote))
        .expect("best vote for 0-prevote must exist");
    assert_eq!(
        best_vote.all_contributors().len(),
        Validator::TWO_F_PLUS_ONE + 1
    );
}

/// Round 1:
///     * The validator sees a proposal and 2f votes for the proposal and 1 against all proposals. Votes None.
///     * During 0-precommit it sees a new 0-Prevote with 2f+1 prevotes. It must not update its vote.
///         0-precommit fails to produce a decision.
/// Round 2: The proposal from round 1 is re-proposed with VR set and that proposal must produce a decision.
#[test(tokio::test)]
async fn it_accepts_vr_proposal() {
    // Validator shall not propose.
    let (proposer, mut observe_receiver) = create_validator(vec![false, false], vec![]);
    let (mut sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 0, None, true);
    // Proposal must be accepted
    let _original_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    let original_prevote = expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f for the proposal and 1 against.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![
            (Some(0), 0..Validator::TWO_F_PLUS_ONE - 1),
            (
                None,
                Validator::TWO_F_PLUS_ONE..Validator::TWO_F_PLUS_ONE + 1,
            ),
        ],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // Wait for the improvement timeout to expire.
    let _state = await_state(&mut tendermint).await;

    // Start 0-precommit aggregation. As the vote could improve an await is necessary here, to wait the timeout.
    expect_nothing_observed(&mut observe_receiver);
    let recoverable_state = expect_state(&mut tendermint);
    let original_precommit = expect_observe_aggregate(&mut observe_receiver);
    // Make sure the node has not voted for the Proposal, as it only has 2f votes yet.
    assert_eq!(
        recoverable_state.votes.get(&(0, Step::Precommit)),
        Some(&None)
    );

    // Vote against all proposals in 0-precommit.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See The aggregate. Advance to 1-propose as 0-precommit is conclusive.
    let _state = expect_state(&mut tendermint);

    // Waiting for the proposal of round 1
    assert_poll_pending(&mut tendermint);

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 1, Some(0), true);
    // Proposal must be accepted
    let _proposal_with_vr = expect_proposal(&mut tendermint, Acceptance::Accept);
    // Pending state containing the vote.
    let state = expect_state(&mut tendermint);
    assert_eq!(
        state
            .round_proposals
            .get(&1)
            .expect("proposal set must exist")
            .iter()
            .next(),
        Some((&0, &(Some(0), true))),
    );
    // expect_observe_aggregate(&mut observe_receiver);
    // As the node has not seen an updated vote, there is not a 2f+1 majority for Some(0) in vr= 0.
    // Thus the node must vote None.
    assert_eq!(state.votes.get(&(1, Step::Prevote)), Some(&None));
    expect_nothing_observed(&mut observe_receiver);

    // From here on it is assumed that restarting from state works, without violating the protocol.

    // recreate the node just before it received the updated 0-precommit aggregate.
    let (mut sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        Some(recoverable_state),
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Make sure the node is not doing anything after starting.
    assert_poll_pending(&mut tendermint);
    // Both aggregations 0-prevote and 0-precommit haven been restarted.
    // Potential TODO: remove ordered requirement here. Implementation gives them ordered, but should not be a requirement.
    let prevote = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(prevote.tag, original_prevote.tag);
    assert_eq!(prevote.aggregation, original_prevote.aggregation);
    let precommit = expect_observe_aggregate(&mut observe_receiver);
    assert_eq!(precommit.tag, original_precommit.tag);
    assert_eq!(precommit.aggregation, original_precommit.aggregation);

    // This time around also send an updated vote for 0-prevote.
    // Vote with 2f+1 for the proposal int he already elapsed 0-prevote.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![
            (Some(0), 0..Validator::TWO_F_PLUS_ONE),
            (
                None,
                Validator::TWO_F_PLUS_ONE..Validator::TWO_F_PLUS_ONE + 1,
            ),
        ],
    );
    // Vote against all proposals in 0-precommit.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See both new aggregates. Advance to 1-propose as 0-precommit is conclusive.
    let _state = expect_state(&mut tendermint);

    // Waiting for the proposal of round 1
    assert_poll_pending(&mut tendermint);

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 1, Some(0), true);
    // Proposal must be accepted
    let _proposal_with_vr = expect_proposal(&mut tendermint, Acceptance::Accept);
    // Pending state containing the vote.
    let state = expect_state(&mut tendermint);
    assert_eq!(
        state
            .round_proposals
            .get(&1)
            .expect("proposal set must exist")
            .iter()
            .next(),
        Some((&0, &(Some(0), true))),
    );
    assert_eq!(state.votes.get(&(1, Step::Prevote)), Some(&Some(0)));

    expect_nothing_observed(&mut observe_receiver);
}

/// The validator sees a proposal and a good prevote aggregation, but the precommit aggregations times out without a result.
/// At a later time it sees the good precommit and should produce a decision even though it has progressed into a later round.
#[test(tokio::test)]
async fn it_accepts_late_polka() {
    // Validator shall not propose.
    let (proposer, mut observe_receiver) = create_validator(vec![false, false], vec![]);
    let (mut sender, receiver) = mpsc::channel(10);

    let mut tendermint = Tendermint::new(
        proposer.clone(),
        None,
        ReceiverStream::new(receiver).boxed(),
        stream::iter(vec![]).boxed(),
    );

    // Send good proposal for round 0
    send_proposal(&mut sender, 0, 0, None, true);
    // Proposal must be accepted
    let _original_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    let _original_prevote = expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (0, Step::Prevote),
        vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // Start 0-precommit aggregation. As the vote could improve an await is necessary here, to wait the timeout.
    let _recoverable_state = expect_state(&mut tendermint);
    let _precommit = expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f for the proposal and 1 against.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![
            (Some(0), 0..Validator::TWO_F_PLUS_ONE - 1),
            (
                None,
                Validator::TWO_F_PLUS_ONE + 1..Validator::TWO_F_PLUS_ONE + 2,
            ),
        ],
    );

    // See the vote
    let _state = expect_state(&mut tendermint);

    // Wait for the improvement timeout to expire.
    let _state = await_state(&mut tendermint).await;

    // Send good proposal for round 1
    send_proposal(&mut sender, 1, 1, None, true);
    // Proposal must be accepted
    let _original_proposal = expect_proposal(&mut tendermint, Acceptance::Accept);

    // Pending state, as previously only the proposal acceptance was returned.
    let _state = expect_state(&mut tendermint);

    // Starting 0-prevote
    expect_nothing_observed(&mut observe_receiver);
    let _state = expect_state(&mut tendermint);
    let _prevote_1 = expect_observe_aggregate(&mut observe_receiver);

    // Vote with 2f+1 for the proposal.
    aggregate(
        &proposer,
        (0, Step::Precommit),
        vec![
            (Some(0), 0..Validator::TWO_F_PLUS_ONE),
            (
                None,
                Validator::TWO_F_PLUS_ONE + 1..Validator::TWO_F_PLUS_ONE + 2,
            ),
        ],
    );

    // Witness improved precommit, produce decision even though it is in round 1 now.
    let decision = expect_decision(&mut tendermint);

    assert_eq!(decision.proposal.0, 0);
    assert_eq!(decision.inherents.0, 0);
    assert_eq!(decision.round, 0);
    assert_eq!(decision.sig.len(), Validator::TWO_F_PLUS_ONE);
}
