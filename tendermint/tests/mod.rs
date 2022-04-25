#[macro_use]
extern crate beserial_derive;

use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use futures::{FutureExt, StreamExt};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_primitives::policy::{SLOTS, TWO_F_PLUS_ONE};
use nimiq_tendermint::*;
use nimiq_test_log::test;
use std::collections::BTreeMap;
use std::io;

// We need to create a type of proposal just for testing. The first field is meant to be the actual
// proposal value (eg: proposal 1) while the second field can be used to represent the round when a
// proposal is proposed.
// Note that the second field is invisible to Tendermint (as can be seen from the following trait
// implementations), it is only meant to uniquely identify a proposal during testing.
#[derive(Copy, Clone, Debug, Eq, Serialize, Deserialize)]
pub struct TestProposal(u32, u32);

// We only check equality for the first field.
impl PartialEq for TestProposal {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// We only serialize the first field. Since char doesn't implement serialize, we opt to convert it
// to a u32.
impl SerializeContent for TestProposal {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = self.0.serialize(writer)?;
        Ok(size)
    }
}

impl Hash for TestProposal {}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestProposalCache(u32);

// We use this struct to mimic a validator. It determines which messages a single validator sees.
#[derive(Clone)]
pub struct TestValidator {
    // This is the round when this validator will be the proposer.
    proposer_round: u32,
    // This vector stores the proposal messages that this validator sees for each round. To be more
    // clear, the i-th element of the vector contains the proposal message for the i-th round.
    // Each proposal message is a tuple consisting of: 1) a boolean flag stating if the message is a
    // timeout, 2) the proposal and 3) the valid round of the proposer.
    proposal_rounds: Vec<(bool, TestProposal, Option<u32>)>,
    // This vector stores the prevote aggregation messages that this validator sees for each round.
    // To be more clear, the i-th element of the vector contains the prevote aggregation message for
    // the i-th round.
    // Each proposal message is a tuple consisting of: 1) a boolean flag stating if the message is
    // NewRound, 2) the number of votes for the proposal 'A', 3) the number of votes for the
    // proposal 'B' and 4) the number of votes for Nil.
    agg_prevote_rounds: Vec<(bool, u16, u16, u16)>,
    // Same as above but for precommit aggregation messages.
    agg_precommit_rounds: Vec<(bool, u16, u16, u16)>,
    // Same as `agg_prevote_rounds` but this vector is accessed by the `get_aggregation` method
    // instead of being accessed by the `broadcast_and_aggregate` method like the two above vectors.
    get_agg_rounds: Vec<(bool, u16, u16, u16)>,
}

// This is the implementation of TendermintOutsideDeps for our test validator. This determines how
// the validator will process the messages that it receives.
#[async_trait]
impl TendermintOutsideDeps for TestValidator {
    // The proposal type is obviously our TestProposal type.
    type ProposalTy = TestProposal;
    // The proposalCacheTy is going to be TestProposalCache
    type ProposalCacheTy = TestProposalCache;
    // Any hash function, doesn't matter which.
    type ProposalHashTy = Blake2bHash;
    // We never verify the proofs inside Tendermint so we can just use the empty type.
    type ProofTy = ();
    // The result would normally be a combination of the proposal and the proof. But since our proof
    // is just the empty type, our result is just the TestProposal.
    type ResultTy = TestProposal;

    fn block_height(&self) -> u32 {
        0
    }

    fn initial_round(&self) -> u32 {
        0
    }

    // We never call this on tests. Needs to be implemented if we want to use it.
    fn verify_state(
        &self,
        _state: &TendermintState<
            Self::ProposalTy,
            Self::ProposalCacheTy,
            Self::ProposalHashTy,
            Self::ProofTy,
        >,
    ) -> bool {
        true
    }

    fn is_our_turn(&self, round: u32) -> bool {
        self.proposer_round == round
    }

    // When it is our turn to propose, the proposal message in `proposal_rounds` is used instead to
    // give us the value that we will propose.
    fn get_value(
        &mut self,
        round: u32,
    ) -> Result<(Self::ProposalTy, Self::ProposalCacheTy), TendermintError> {
        Ok((
            self.proposal_rounds[round as usize].1,
            TestProposalCache(self.proposal_rounds[round as usize].1 .0),
        ))
    }

    fn assemble_block(
        &self,
        _round: u32,
        proposal: Self::ProposalTy,
        proposal_cache: Option<Self::ProposalCacheTy>,
        _proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError> {
        assert!(proposal_cache.is_some());
        assert_eq!(proposal.0, proposal_cache.unwrap().0);
        Ok(proposal)
    }

    async fn broadcast_proposal(
        self,
        _round: u32,
        _proposal: Self::ProposalTy,
        _valid_round: Option<u32>,
    ) -> Result<Self, TendermintError> {
        Ok(self)
    }

    async fn await_proposal(
        self,
        round: u32,
    ) -> Result<
        (
            Self,
            ProposalResult<Self::ProposalTy, Self::ProposalCacheTy>,
        ),
        TendermintError,
    > {
        if self.proposal_rounds[round as usize].0 {
            // If the timeout flag is set, return timeout.
            Ok((self, ProposalResult::Timeout))
        } else {
            let proposal = ProposalResult::Proposal(
                (
                    self.proposal_rounds[round as usize].1,
                    TestProposalCache(self.proposal_rounds[round as usize].1 .0),
                ),
                self.proposal_rounds[round as usize].2,
            );

            Ok((self, proposal))
        }
    }

    // Note that this function returns an AggregationResult, not VoteResult. AggregationResult can
    // only be an Aggregation (containing the raw votes) or a NewRound.
    async fn broadcast_and_aggregate(
        self,
        round: u32,
        step: Step,
        _proposal_hash: Option<Self::ProposalHashTy>,
    ) -> Result<(Self, AggregationResult<Self::ProposalHashTy, Self::ProofTy>), TendermintError>
    {
        // Calculate the hashes for the proposals 'A' and 'B'.
        let a_hash = TestProposal(0, 0).hash();
        let b_hash = TestProposal(1, 0).hash();

        match step {
            Step::Prevote => {
                if self.agg_prevote_rounds[round as usize].0 {
                    // If the new round flag is set, return NewRound for the next round.
                    Ok((self, AggregationResult::NewRound(round + 1)))
                } else {
                    // Otherwise, save the votes for 'A', 'B' and Nil in a BTreeMap and return it.
                    let mut agg = BTreeMap::new();
                    agg.insert(
                        Some(a_hash),
                        ((), self.agg_prevote_rounds[round as usize].1),
                    );
                    agg.insert(
                        Some(b_hash),
                        ((), self.agg_prevote_rounds[round as usize].2),
                    );
                    agg.insert(None, ((), self.agg_prevote_rounds[round as usize].3));
                    Ok((self, AggregationResult::Aggregation(agg)))
                }
            }
            Step::Precommit => {
                if self.agg_precommit_rounds[round as usize].0 {
                    // If the new round flag is set, return NewRound for the next round.
                    Ok((self, AggregationResult::NewRound(round + 1)))
                } else {
                    // Otherwise, save the votes for 'A', 'B' and Nil in a BTreeMap and return it.
                    let mut agg = BTreeMap::new();
                    agg.insert(
                        Some(a_hash),
                        ((), self.agg_precommit_rounds[round as usize].1),
                    );
                    agg.insert(
                        Some(b_hash),
                        ((), self.agg_precommit_rounds[round as usize].2),
                    );
                    agg.insert(None, ((), self.agg_precommit_rounds[round as usize].3));
                    Ok((self, AggregationResult::Aggregation(agg)))
                }
            }
            _ => unreachable!(),
        }
    }

    // Same as ´broadcast_and_aggregate´ but here we only check for prevotes.
    async fn get_aggregation(
        self,
        round: u32,
        _step: Step,
    ) -> Result<(Self, AggregationResult<Self::ProposalHashTy, Self::ProofTy>), TendermintError>
    {
        // Calculate the hashes for the proposals 'A' and 'B'.
        let a_hash = TestProposal(0, 0).hash();
        let b_hash = TestProposal(1, 0).hash();

        if self.get_agg_rounds[round as usize].0 {
            // If the new round flag is set, return NewRound for the next round.
            Ok((self, AggregationResult::NewRound(round + 1)))
        } else {
            // Otherwise, save the votes for 'A', 'B' and Nil in a BTreeMap and return it.
            let mut agg = BTreeMap::new();
            agg.insert(Some(a_hash), ((), self.get_agg_rounds[round as usize].1));
            agg.insert(Some(b_hash), ((), self.get_agg_rounds[round as usize].2));
            agg.insert(None, ((), self.get_agg_rounds[round as usize].3));
            Ok((self, AggregationResult::Aggregation(agg)))
        }
    }

    fn hash_proposal(
        &self,
        proposal: Self::ProposalTy,
        _proposal_cache: Self::ProposalCacheTy,
    ) -> Self::ProposalHashTy {
        proposal.hash()
    }

    fn get_background_task(&mut self) -> futures::future::BoxFuture<'static, ()> {
        futures::future::pending().boxed()
    }
}

// This is the function that runs the Tendermint stream to completion. It takes as input a
// TestValidator and optionally a TendermintState (only needed if are recovering to a previous
// state). It also takes as input the proposal against which the result of our Stream will be
// compared.
async fn tendermint_loop(
    deps: TestValidator,
    state_opt: Option<
        TendermintState<
            <TestValidator as TendermintOutsideDeps>::ProposalTy,
            <TestValidator as TendermintOutsideDeps>::ProposalCacheTy,
            <TestValidator as TendermintOutsideDeps>::ProposalHashTy,
            <TestValidator as TendermintOutsideDeps>::ProofTy,
        >,
    >,
    reference_proposal: TestProposal,
) -> Result<(), TestValidator> {
    // Get the stream.
    let mut tendermint = Tendermint::new(deps, state_opt)?;

    // Start running the stream. It runs until it returns a Result.
    while let Some(value) = tendermint.next().await {
        match value {
            // If it returns a Result we compare it with the reference proposal.
            // We need to deconstruct TestProposal because we implemented a PartialEq trait that
            // ignores the last field.
            TendermintReturn::Result(result) => {
                assert_eq!(result.0, reference_proposal.0);
                assert_eq!(result.1, reference_proposal.1);
                break;
            }
            // Whenever there is a state update we print it. That way if the test fails we have log
            // of what the validator did.
            TendermintReturn::StateUpdate(state) => println!("{:#?}\n", state),
            // We can never get an error because our implementation of TendermintOutsideDeps doesn't
            // returns errors.
            TendermintReturn::Error(_) => unreachable!(),
        }
    }
    Ok(())
}

#[tokio::test]
async fn it_can_reenter_from_state() {
    // try to capture state transitions
    // first round is timedout proposal with Nil votes in both aggregation steps.
    // second round is a proposal and a NewRound for the aggregate prevote
    // third round is a proposal with a new round for aggregate precommit
    // forth round is a proposal with slots prevotes but 0 precommits. This should lock.
    // fifth round is (1,3) again with VR(3) SLOTS prevotes and 0 precommits
    // sixth this node is proposing, it should propose what was proposed a round ago (and two rounds ago)
    let proposer = TestValidator {
        proposer_round: 5,
        proposal_rounds: vec![
            (true, TestProposal(0, 0), None),
            (false, TestProposal(0, 1), None),
            (false, TestProposal(0, 2), None),
            (false, TestProposal(1, 3), None),
            (false, TestProposal(1, 3), Some(3)),
            (false, TestProposal(0, 5), None), // irrelevant the node should propose (1,3)
        ],
        agg_prevote_rounds: vec![
            (false, 0, 0, SLOTS),
            (true, 0, 0, SLOTS),
            (false, 0, 0, SLOTS),
            (false, 0, SLOTS, 0),
            (false, 0, SLOTS, 0),
            (false, 0, SLOTS, 0),
        ],
        agg_precommit_rounds: vec![
            (false, 0, 0, SLOTS),
            (false, 0, 0, SLOTS),
            (true, 0, 0, SLOTS),
            (false, 0, 0, SLOTS),
            (false, 0, 0, SLOTS),
            (false, 0, SLOTS, 0),
        ],
        get_agg_rounds: vec![
            (false, 0, 0, SLOTS),
            (false, 0, 0, SLOTS),
            (false, 0, SLOTS, 0),
            (false, 0, SLOTS, 0),
            (false, 0, 0, SLOTS),
            (false, 0, 0, SLOTS),
        ],
    };

    let mut state = None;
    let mut next_state = None;

    if let Ok(mut tendermint) = Tendermint::new(proposer.clone(), None) {
        if let Some(value) = tendermint.next().await {
            match value {
                TendermintReturn::Result(_) => {
                    unreachable!();
                }
                TendermintReturn::StateUpdate(st) => next_state = Some(st),
                TendermintReturn::Error(e) => {
                    println!("Error: {:?}", e);
                    unreachable!();
                }
            }
        }
    } else {
        panic!("Could not create Tendermint instance.");
    }

    loop {
        // create Tendermint instance without a state.
        if let Ok(mut tendermint) = Tendermint::new(proposer.clone(), state.take()) {
            if let Some(value) = tendermint.next().await {
                match value {
                    TendermintReturn::Result(result) => {
                        // make sure the accepted proposal is correct.
                        assert_eq!(result.0, 1);
                        assert_eq!(result.1, 3);
                        break;
                    }
                    // Whenever there is a state update we print it. That way if the test fails we have log
                    // of what the validator did.
                    TendermintReturn::StateUpdate(st) => {
                        // compare state with the next state we are expecting
                        let st = Some(st);
                        println!("{:?}\n", &st);
                        assert_eq!(&st, &next_state);
                        state = st;

                        // produce the next state to be checked against next loop cycle.
                        if let Some(value) = tendermint.next().await {
                            match value {
                                TendermintReturn::Result(result) => {
                                    // make sure the accepted proposal is correct.
                                    assert_eq!(result.0, 1);
                                    assert_eq!(result.1, 3);
                                }
                                TendermintReturn::StateUpdate(st) => next_state = Some(st),
                                TendermintReturn::Error(e) => {
                                    println!("Error: {:?}", e);
                                    unreachable!();
                                }
                            }
                        }
                    }
                    // We can never get an error because our implementation of TendermintOutsideDeps doesn't
                    // returns errors.
                    TendermintReturn::Error(e) => {
                        println!("Error: {:?}", e);
                        unreachable!();
                    }
                }
            }
        } else {
            panic!("Could not create Tendermint instance.");
        }
    }
}

#[tokio::test]
async fn it_does_not_unlock() {
    let val = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal(0, 0), None),
            (true, TestProposal(1, 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, 0, SLOTS, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, SLOTS), (false, 0, 0, SLOTS)],
        get_agg_rounds: vec![(false, SLOTS, 0, 0), (false, 0, SLOTS, 0)],
    };

    if let Ok(mut tendermint) = Tendermint::new(val, None) {
        // process until the first AggregatePreCommit state to check that the node is indeed locked
        loop {
            if let Some(ret) = tendermint.next().await {
                match ret {
                    TendermintReturn::Error(err) => {
                        println!("Error: {:?}", err);
                        unreachable!();
                    }
                    TendermintReturn::Result(res) => {
                        println!("Result: {:?}", res);
                        unreachable!();
                    }
                    TendermintReturn::StateUpdate(state) => {
                        if state.current_checkpoint == Checkpoint::AggregatePreCommit {
                            assert!(state.locked.is_some());
                            let locked = state.locked.as_ref().unwrap();
                            assert_eq!(state.round, 0);
                            assert_eq!(locked.proposal.0, 0);
                            assert_eq!(locked.proposal.1, 0);
                            break;
                        }
                    }
                }
            }
        }

        // process further until we reach the next precommit. This node should not see the proposal and thus will
        // be unable to unlock even if it sees >2f+1 Prevotes
        loop {
            if let Some(ret) = tendermint.next().await {
                match ret {
                    TendermintReturn::Error(err) => {
                        println!("Error: {:?}", err);
                        unreachable!();
                    }
                    TendermintReturn::Result(res) => {
                        println!("Result: {:?}", res);
                        unreachable!();
                    }
                    TendermintReturn::StateUpdate(state) => {
                        if state.current_checkpoint == Checkpoint::AggregatePreCommit {
                            assert!(state.locked.is_some());
                            assert_eq!(state.round, 1);
                            let locked = state.locked.as_ref().unwrap();
                            assert_eq!(locked.proposal.0, 0);
                            assert_eq!(locked.proposal.1, 0);
                            break;
                        }
                    }
                }
            }
        }
    } else {
        panic!("Could not create Tendermint instance.");
    }
}

// Simple test where everything goes normally.
#[test(tokio::test)]
async fn everything_works() {
    // From the perspective of the proposer.
    let proposer = TestValidator {
        proposer_round: 0,
        proposal_rounds: vec![(false, TestProposal(0, 0), None)],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(proposer, None, TestProposal(0, 0))
        .await
        .is_ok());

    // From the perspective of another validator.
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![(false, TestProposal(0, 0), None)],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 0))
        .await
        .is_ok());
}

// The first proposer does not send a proposal but the second one does.
#[test(tokio::test)]
async fn no_proposal() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (true, TestProposal(0, 0), None),
            (false, TestProposal(0, 1), None),
        ],
        agg_prevote_rounds: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 1))
        .await
        .is_ok());
}

// Our validator doesn't see any messages for the entire first round.
// NOTE: When we want a prevote or precommit aggregation to be a Timeout, we send 0 votes for 'A',
// 'B' and Nil. Technically this is incorrect since any aggregation will always have at least 2f+1
// votes. But this trick will produce the same result and it's simpler to write.
#[test(tokio::test)]
async fn all_timeouts() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (true, TestProposal(0, 0), None),
            (false, TestProposal(0, 1), None),
        ],
        agg_prevote_rounds: vec![(false, 0, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 1))
        .await
        .is_ok());
}

// We don't have enough prevotes for the first proposal we received.
#[test(tokio::test)]
async fn not_enough_prevotes() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal(0, 0), None),
            (false, TestProposal(0, 1), None),
        ],
        agg_prevote_rounds: vec![(false, TWO_F_PLUS_ONE - 1, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 1))
        .await
        .is_ok());
}

// We don't have enough precommits for the first proposal we received.
#[test(tokio::test)]
async fn not_enough_precommits() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal(0, 0), None),
            (false, TestProposal(0, 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, TWO_F_PLUS_ONE - 1, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 1))
        .await
        .is_ok());
}

// Our validator locks on the first round proposal, which doesn't complete, and then needs to
// rebroadcast the proposal since he is the next proposer.
#[test(tokio::test)]
async fn locks_and_rebroadcasts() {
    let validator = TestValidator {
        proposer_round: 1,
        proposal_rounds: vec![
            (false, TestProposal(0, 0), None),
            // This is not actually sent. Validator Rebroadcasts TestProposal('A', 0) since that is
            // what it has stored.
            (false, TestProposal(0, 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 0))
        .await
        .is_ok());
}

// Our validator locks on the first round proposal, which doesn't complete, and then needs to unlock
// on the second round when that proposal gets 2f+1 prevotes.
#[test(tokio::test)]
async fn locks_and_unlocks() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal(0, 0), None),
            (false, TestProposal(1, 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, 0, SLOTS, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, 0, SLOTS, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(1, 1))
        .await
        .is_ok());
}

// Our validator doesn't receive any prevotes for the proposal but then receives enough precommits
// and is forced to accept the proposal.
#[test(tokio::test)]
async fn forced_commit() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![(false, TestProposal(0, 0), None)],
        agg_prevote_rounds: vec![(false, 0, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 0))
        .await
        .is_ok());
}

// The first and second proposals don't get enough prevotes or precommits, and the third proposal
// is a rebroadcast of the first one.
#[test(tokio::test)]
async fn past_proposal() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal(0, 0), None),
            (false, TestProposal(1, 1), None),
            (false, TestProposal(0, 2), Some(0)),
        ],
        agg_prevote_rounds: vec![(false, 0, 0, 0), (false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![(false, SLOTS, 0, 0)],
    };

    assert!(tendermint_loop(validator, None, TestProposal(0, 2))
        .await
        .is_ok());
}

#[test(tokio::test)]
async fn it_can_assemble_block_from_state() {
    let val = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![(false, TestProposal(0, 0), None)],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![(false, SLOTS, 0, 0)],
    };

    // First, make the validator lock itself on a value but then drop the instance. Keep the state.
    let prev_state = if let Ok(mut tendermint) = Tendermint::new(val.clone(), None) {
        // Poll the first instance until it results in an AggregatePreCommit state
        loop {
            if let Some(ret) = tendermint.next().await {
                match ret {
                    TendermintReturn::Error(err) => {
                        println!("Error: {:?}", err);
                        unreachable!();
                    }
                    TendermintReturn::Result(res) => {
                        println!("Result: {:?}", res);
                        unreachable!();
                    }
                    TendermintReturn::StateUpdate(state) => {
                        if state.current_checkpoint == Checkpoint::AggregatePreCommit {
                            // make sure the node is locked.
                            assert!(state.locked.is_some());
                            let locked = state.locked.as_ref().unwrap();
                            assert_eq!(state.round, 0);
                            assert_eq!(locked.proposal.0, 0);
                            assert_eq!(locked.proposal.1, 0);
                            break state;
                        }
                    }
                }
            }
        }
    } else {
        panic!("Could not create Tendermint instance.");
    };

    // Second, recreate the validator with the locked state and drive it to completion.
    if let Ok(mut tendermint) = Tendermint::new(val.clone(), Some(prev_state)) {
        // Poll the new instance once, which must create the block.
        if let Some(ret) = tendermint.next().await {
            match ret {
                TendermintReturn::Error(err) => {
                    println!("Error: {:?}", err);
                    unreachable!();
                }
                TendermintReturn::Result(res) => {
                    assert_eq!(res.0, val.proposal_rounds[0].1 .0);
                    assert_eq!(res.1, val.proposal_rounds[0].1 .1);
                }
                TendermintReturn::StateUpdate(_state) => {
                    unreachable!();
                }
            }
        }
    } else {
        panic!("Could not create Tendermint instance.");
    }
}
