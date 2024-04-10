use std::{collections::btree_map::Entry, task::Context};

use futures::{future::FutureExt, stream::StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    protocol::{Aggregation, Protocol},
    utils::{Return, Step},
    Tendermint,
};

impl<TProtocol: Protocol> Tendermint<TProtocol> {
    pub(crate) fn aggregate(&mut self, cx: &mut Context<'_>) -> Option<Return<TProtocol>> {
        // create  the aggregation if it does not exist yet.
        self.create_aggregation();

        let round_and_step = (self.state.current_round, self.state.current_step);

        // get the best aggregate for the identifier. If it does not exist, there is nothing to check.
        let current_best = self.state.best_votes.get(&round_and_step)?;

        // Get the number of total contributors.
        let total_contributors = current_best.all_contributors().len();

        // If the number of total contributors is below 2f+1 nothing can happen, so check that first.
        // Needed even though the loop will check this too, as in this case the timeout is not started, whereas after this condition
        // evaluated to false the timeout must always be started if no final result is reached.
        if total_contributors < TProtocol::TWO_F_PLUS_ONE {
            return None;
        }

        // Copy the total such that it can be modified.
        let mut remaining_contributor_count = total_contributors;

        // Keep track of whether the aggregate can improve to a positive result or not. If it cannot then no timeout would be necessary.
        // The aggregate can improve if` for any single proposal` there are enough votes such that the remaining, uncast votes would
        // elevate it over the 2f+1 threshold if cast for that proposal.
        // More generally speaking, if there have not been more than f votes for anything else, a proposal can still reach 2f+1
        let mut can_improve = false;

        // Get the set of proposals, with their VR for the current round.
        // Note that this set might be empty.
        let proposals = self
            .state
            .round_proposals
            .entry(self.state.current_round)
            .or_default();

        // Go over the proposals known to the node for this round.
        // As they are the only ones that can result in an action all others can be ignored.
        // Those which bear any chance of reaching a result will be requested during aggregation acquisition.
        for proposal_hash in proposals.keys().cloned() {
            let proposal_contributor_count =
                current_best.contributors_for(Some(&proposal_hash)).len();
            // If there are not enough votes, continue with the next
            if proposal_contributor_count < TProtocol::TWO_F_PLUS_ONE {
                // The vote can improve if there is a proposal with < 2f+1 votes currently where also
                // all of the other votes combined do not exceed f
                can_improve |=
                    total_contributors - proposal_contributor_count < TProtocol::F_PLUS_ONE;
                // Keep the remaining contributor count accurate.
                remaining_contributor_count -= proposal_contributor_count;
                continue;
            }

            // The proposal has at least 2f+1 votes.
            log::debug!(
                ?round_and_step,
                proposal = ?proposal_hash,
                "Aggregation resulted in Block polka",
            );
            self.on_polka(proposal_hash);

            // Reset timeout.
            self.timeout = None;
            // Yield state.
            return Some(Return::Update(self.state.clone()));
        }

        // Also check the vote for None as it could be conclusive.
        let none_contributor_count = current_best.contributors_for(None).len();

        // Since checking if None has 2f+1 results in the same action as no result being able to improve to 2f+1,
        // they are handled together.
        if none_contributor_count >= TProtocol::TWO_F_PLUS_ONE {
            // Vote against all proposals, as None has 2f+1 votes.
            log::debug!(?round_and_step, "Aggregation resulted in None polka",);
            self.on_none_polka();

            // Reset timeout.
            self.timeout = None;
            // Yield state.
            return Some(Return::Update(self.state.clone()));
        }

        // Keep the remaining contributor count accurate.
        remaining_contributor_count -= none_contributor_count;

        // If none of the proposals checked can improve to 2f+1 there is the additional chance, that there might be an unknown proposal
        // which can (or already has) reached 2f+1 votes. As it is unknown it can not be voted for, but it should have been requested thus
        // waiting for the timeout could lead to receiving that proposal.
        // Only if neither known proposals can improve, nor is there enough vote power left to reach a conclusion the timeout is skipped.

        if !can_improve && total_contributors - remaining_contributor_count >= TProtocol::F_PLUS_ONE
        {
            // Vote against all proposals, as None has 2f+1 votes.
            log::debug!(?round_and_step, "Aggregation resulted in None polka",);
            self.on_none_polka();

            // Reset timeout.
            self.timeout = None;
            // Yield state.
            return Some(Return::Update(self.state.clone()));
        }

        // No final result was reached, even though the aggregation has at least 2f+1 total votes.
        // Start the timeout such that a better result is waited upon for the duration.
        self.start_timeout();

        // Check if the timeout elapsed. If so the result must be returned, even though it might still improve.
        if self.timeout.as_mut().unwrap().poll_unpin(cx).is_ready() {
            log::debug!("Aggregation timed out without final result.");
            self.on_none_polka();

            // Reset timeout.
            self.timeout = None;
            // Yield state.
            return Some(Return::Update(self.state.clone()));
        }

        // Timeout is not expired. Return None.
        None
    }

    /// For the current round and step as denoted within `self.state` this will perform all necessary
    /// action to advance to the next state while having seen 2f+1 votes for the known proposal with `proposal_hash`
    /// as its hash.
    ///
    /// As precommit aggregations with 2f+1 votes result in a decision being produced and as those are produced
    /// while polling ongoing aggregations that match arm is unreachable!() here.
    ///
    /// This cannot fail.
    fn on_polka(&mut self, proposal_hash: TProtocol::ProposalHash) {
        // While for Step::Propose this is trivial for Step::Precommit this seems unintuitive.
        // However seeing a polka for a precommit will produce a decision. That already happens when
        // collecting new aggregates from ongoing aggregations. Thus here only prevotes are handled.
        match self.state.current_step {
            Step::Prevote => {
                // Advance to precommit step.
                self.state.current_step = Step::Precommit;

                // Vote for the proposal in the upcoming precommit.
                assert!(self
                    .state
                    .votes
                    .insert(
                        (self.state.current_round, self.state.current_step),
                        Some(proposal_hash.clone())
                    )
                    .is_none());

                // Since the node will vote to commit, it must lock itself.
                // Note that valid will not be set, as that must happen during processing ALL aggregations.
                // voting for a proposal in precommit is NOT the only way of setting valid.
                self.state.locked = Some((self.state.current_round, proposal_hash));
            }
            Step::Precommit => unreachable!(),
            Step::Propose => {
                panic!("current_step must not be Step::Propose when calling aggregate()")
            }
        }
    }

    /// Advances the current step and state to the next appropriate state with the current aggregation not having
    /// produced a proposal with 2f+1 votes.
    ///
    /// This cannot fail.
    fn on_none_polka(&mut self) {
        match self.state.current_step {
            Step::Prevote => {
                // Advance step to precommit.
                self.state.current_step = Step::Precommit;
                // Vote for None in upcoming precommit.
                assert!(self
                    .state
                    .votes
                    .insert((self.state.current_round, self.state.current_step), None)
                    .is_none());
            }
            Step::Precommit => {
                // Start the next round.
                self.state.current_round += 1;
                // Set step to propose.
                self.state.current_step = Step::Propose;
                // Remove all future contributions for the round that is about to start.
                self.future_contributions
                    .retain(|round, _contributors| round > &self.state.current_round);
            }
            Step::Propose => unreachable!(),
        }
    }

    /// Creates an aggregation for `(self.state.current_round, self.state.current_step)`. If that aggregation does already exist,
    /// as indicated by the presence of `id` in `self.aggregation_senders` it has no effect.
    ///
    /// The vote for this aggregation must exist in `self.state.votes` otherwise this function panics.
    fn create_aggregation(&mut self) {
        // Create identifier for the current aggregation
        let id = (self.state.current_round, self.state.current_step);

        // create the aggregation if it does not exist yet
        if let Entry::Vacant(entry) = self.aggregation_senders.entry(id) {
            log::debug!(?id, "Creating Aggregation");
            // Retrieve the vote this node is going to take.
            let vote = self.state.votes.get(&id).expect("").clone();
            // create the corresponding contribution necessary to start an aggregation
            // let contribution = self.dependencies.create_contribution(self.state.current_round, self.state.current_step, vote);
            // create the channel to dispatch level updates over
            let (sender, receiver) = mpsc::channel(100);
            // create the aggregation
            let aggregation = self.protocol.create_aggregation(
                self.state.current_round,
                self.state.current_step,
                vote,
                ReceiverStream::new(receiver).boxed(),
            );

            // insert the sender into the map, such that level updates for the aggregation can be dispatched
            entry.insert(sender);
            // add the aggregation to the SelectAll of all aggregations
            self.aggregations
                .push(aggregation.map(move |item| (id, item)).boxed());
        }
    }
}
