use async_trait::async_trait;
use beserial::Serialize;
use futures::{FutureExt, StreamExt};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_primitives::policy::{SLOTS, TWO_THIRD_SLOTS};
use nimiq_tendermint::*;
use std::collections::BTreeMap;
use std::io;

// We need to create a type of proposal just for testing. The first field is meant to be the actual
// proposal value (eg: proposal A) while the second field can be used to represent the round when a
// proposal is proposed.
// Note that the second field is invisible to Tendermint (as can be seen from the following trait
// implementations), it is only meant to uniquely identify a proposal during testing.
#[derive(Copy, Clone, Debug, Eq)]
pub struct TestProposal(char, u32);

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
        let size = self.0.to_digit(36).unwrap().serialize(writer)?;
        Ok(size)
    }
}

impl Hash for TestProposal {}

// We use this struct to mimic a validator. It determines which messages a single validator sees.
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
    // We never verify the proofs inside Tendermint so we can just use the empty type.
    type ProofTy = ();
    // The result would normally be a combination of the proposal and the proof. But since our proof
    // is just the empty type, our result is just the TestProposal.
    type ResultTy = TestProposal;

    // We never call this on tests. Needs to be implemented if we want to use it.
    fn verify_state(&self, _state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        unimplemented!()
    }

    fn is_our_turn(&self, round: u32) -> bool {
        self.proposer_round == round
    }

    // When it is our turn to propose, the proposal message in `proposal_rounds` is used instead to
    // give us the value that we will propose.
    fn get_value(&mut self, round: u32) -> Result<Self::ProposalTy, TendermintError> {
        Ok(self.proposal_rounds[round as usize].1)
    }

    fn assemble_block(
        &self,
        _round: u32,
        proposal: Self::ProposalTy,
        _proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError> {
        Ok(proposal)
    }

    async fn broadcast_proposal(
        &mut self,
        _round: u32,
        _proposal: Self::ProposalTy,
        _valid_round: Option<u32>,
    ) -> Result<(), TendermintError> {
        Ok(())
    }

    async fn await_proposal(
        &mut self,
        round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError> {
        if self.proposal_rounds[round as usize].0 {
            // If the timeout flag is set, return timeout.
            Ok(ProposalResult::Timeout)
        } else {
            // Otherwise, return the proposal.
            Ok(ProposalResult::Proposal(
                self.proposal_rounds[round as usize].1,
                self.proposal_rounds[round as usize].2,
            ))
        }
    }

    // Note that this function returns an AggregationResult, not VoteResult. AggregationResult can
    // only be an Aggregation (containing the raw votes) or a NewRound.
    async fn broadcast_and_aggregate(
        &mut self,
        round: u32,
        step: Step,
        _proposal: Option<Blake2bHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        // Calculate the hashes for the proposals 'A' and 'B'.
        let a_hash = TestProposal('A', 0).hash();
        let b_hash = TestProposal('B', 0).hash();

        match step {
            Step::Prevote => {
                if self.agg_prevote_rounds[round as usize].0 {
                    // If the new round flag is set, return NewRound for the next round.
                    Ok(AggregationResult::NewRound(round + 1))
                } else {
                    // Otherwise, save the votes for 'A', 'B' and Nil in a BTreeMap and return it.
                    let mut agg = BTreeMap::new();
                    agg.insert(
                        Some(a_hash),
                        ((), self.agg_prevote_rounds[round as usize].1 as usize),
                    );
                    agg.insert(
                        Some(b_hash),
                        ((), self.agg_prevote_rounds[round as usize].2 as usize),
                    );
                    agg.insert(
                        None,
                        ((), self.agg_prevote_rounds[round as usize].3 as usize),
                    );
                    Ok(AggregationResult::Aggregation(agg))
                }
            }
            Step::Precommit => {
                if self.agg_precommit_rounds[round as usize].0 {
                    // If the new round flag is set, return NewRound for the next round.
                    Ok(AggregationResult::NewRound(round + 1))
                } else {
                    // Otherwise, save the votes for 'A', 'B' and Nil in a BTreeMap and return it.
                    let mut agg = BTreeMap::new();
                    agg.insert(
                        Some(a_hash),
                        ((), self.agg_precommit_rounds[round as usize].1 as usize),
                    );
                    agg.insert(
                        Some(b_hash),
                        ((), self.agg_precommit_rounds[round as usize].2 as usize),
                    );
                    agg.insert(
                        None,
                        ((), self.agg_precommit_rounds[round as usize].3 as usize),
                    );
                    Ok(AggregationResult::Aggregation(agg))
                }
            }
            _ => unreachable!(),
        }
    }

    // Same as ´broadcast_and_aggregate´ but here we only check for prevotes.
    async fn get_aggregation(
        &mut self,
        round: u32,
        _step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        // Calculate the hashes for the proposals 'A' and 'B'.
        let a_hash = TestProposal('A', 0).hash();
        let b_hash = TestProposal('B', 0).hash();

        if self.get_agg_rounds[round as usize].0 {
            // If the new round flag is set, return NewRound for the next round.
            Ok(AggregationResult::NewRound(round + 1))
        } else {
            // Otherwise, save the votes for 'A', 'B' and Nil in a BTreeMap and return it.
            let mut agg = BTreeMap::new();
            agg.insert(
                Some(a_hash),
                ((), self.get_agg_rounds[round as usize].1 as usize),
            );
            agg.insert(
                Some(b_hash),
                ((), self.get_agg_rounds[round as usize].2 as usize),
            );
            agg.insert(None, ((), self.get_agg_rounds[round as usize].3 as usize));
            Ok(AggregationResult::Aggregation(agg))
        }
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
            <TestValidator as TendermintOutsideDeps>::ProofTy,
        >,
    >,
    reference_proposal: TestProposal,
) {
    // Get the stream.
    let mut tendermint = Tendermint::new(deps, state_opt);

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
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            // We can never get an error because our implementation of TendermintOutsideDeps doesn't
            // returns errors.
            TendermintReturn::Error(_) => unreachable!(),
        }
    }
}

// Simple test where everything goes normally.
#[tokio::test]
async fn everything_works() {
    // From the perspective of the proposer.
    let proposer = TestValidator {
        proposer_round: 0,
        proposal_rounds: vec![(false, TestProposal('A', 0), None)],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(proposer, None, TestProposal('A', 0)).await;

    // From the perspective of another validator.
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![(false, TestProposal('A', 0), None)],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 0)).await;
}

// The first proposer does not send a proposal but the second one does.
#[tokio::test]
async fn no_proposal() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (true, TestProposal('A', 0), None),
            (false, TestProposal('A', 1), None),
        ],
        agg_prevote_rounds: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 1)).await;
}

// Our validator doesn't see any messages for the entire first round.
// NOTE: When we want a prevote or precommit aggregation to be a Timeout, we send 0 votes for 'A',
// 'B' and Nil. Technically this is incorrect since any aggregation will always have at least 2f+1
// votes. But this trick will produce the same result and it's simpler to write.
#[tokio::test]
async fn all_timeouts() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (true, TestProposal('A', 0), None),
            (false, TestProposal('A', 1), None),
        ],
        agg_prevote_rounds: vec![(false, 0, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 1)).await;
}

// We don't have enough prevotes for the first proposal we received.
#[tokio::test]
async fn not_enough_prevotes() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal('A', 0), None),
            (false, TestProposal('A', 1), None),
        ],
        agg_prevote_rounds: vec![(false, TWO_THIRD_SLOTS - 1, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 1)).await;
}

// We don't have enough precommits for the first proposal we received.
#[tokio::test]
async fn not_enough_precommits() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal('A', 0), None),
            (false, TestProposal('A', 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, TWO_THIRD_SLOTS - 1, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 1)).await;
}

// Our validator locks on the first round proposal, which doesn't complete, and then needs to
// rebroadcast the proposal since he is the next proposer.
#[tokio::test]
async fn locks_and_rebroadcasts() {
    let validator = TestValidator {
        proposer_round: 1,
        proposal_rounds: vec![
            (false, TestProposal('A', 0), None),
            // This is not actually sent. Validator Rebroadcasts TestProposal('A', 0) since that is
            // what it has stored.
            (false, TestProposal('A', 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 0)).await;
}

// Our validator locks on the first round proposal, which doesn't complete, and then needs to unlock
// on the second round when that proposal gets 2f+1 prevotes.
#[tokio::test]
async fn locks_and_unlocks() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal('A', 0), None),
            (false, TestProposal('B', 1), None),
        ],
        agg_prevote_rounds: vec![(false, SLOTS, 0, 0), (false, 0, SLOTS, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, 0, SLOTS, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('B', 1)).await;
}

// Our validator doesn't receive any prevotes for the proposal but then receives enough precommits
// and is forced to accept the proposal.
#[tokio::test]
async fn forced_commit() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![(false, TestProposal('A', 0), None)],
        agg_prevote_rounds: vec![(false, 0, 0, 0)],
        agg_precommit_rounds: vec![(false, SLOTS, 0, 0)],
        get_agg_rounds: vec![],
    };

    tendermint_loop(validator, None, TestProposal('A', 0)).await;
}

// The first and second proposals don't get enough prevotes or precommits, and the third proposal
// is a rebroadcast of the first one.
#[tokio::test]
async fn past_proposal() {
    let validator = TestValidator {
        proposer_round: 99,
        proposal_rounds: vec![
            (false, TestProposal('A', 0), None),
            (false, TestProposal('B', 1), None),
            (false, TestProposal('A', 2), Some(0)),
        ],
        agg_prevote_rounds: vec![(false, 0, 0, 0), (false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        agg_precommit_rounds: vec![(false, 0, 0, 0), (false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_rounds: vec![(false, SLOTS, 0, 0)],
    };

    tendermint_loop(validator, None, TestProposal('A', 2)).await;
}
