use std::task::Context;

use futures::future::FutureExt;

use crate::{Protocol, Return, Step, Tendermint};

impl<TProtocol: Protocol> Tendermint<TProtocol> {
    /// Waits for a proposal to arrive.
    ///
    /// This is independent from processing received proposals, as there might be older or newer proposals incoming as well.
    /// Those proposals do need to be processed, as they might have an effect on `self.state.valid`.
    ///
    /// Validity of received proposals is not checked here, but in processing of the received proposals instead.
    /// This function checks if one or more proposals for `self.state.current_round` exists in the `state.round_proposals`.
    /// If so it will check all necessary conditions for those proposals to decide whether or not to vote for them or not.
    pub(crate) fn await_proposal(&mut self, cx: &mut Context<'_>) -> Option<Return<TProtocol>> {
        // There must not be a vote for this rounds prevote yet.
        assert!(
            !self
                .state
                .votes
                .contains_key(&(self.state.current_round, Step::Prevote)),
            "A vote exists even though the proposal was just now received.",
        );

        // Start the timeout within which the proposal must be received.
        // Resetting the timeout could be done in the respective function within the match
        // but they are kept here to make sure they are removed whenever this function returns a state.
        self.start_timeout();

        // Check if a proposal for the current round exists, and if so if it has a vr attached or not.
        match self
            .state
            .round_proposals
            .entry(self.state.current_round)
            .or_default()
            .iter()
            .next()
            .map(|(hash, (vr, _sig))| (hash.clone(), *vr))
        {
            Some((proposal, Some(vr))) => {
                self.received_proposal_with_vr(proposal, vr);

                self.timeout = None;
                Some(Return::Update(self.state.clone()))
            }
            Some((proposal, None)) => {
                self.received_proposal_without_vr(proposal);

                self.timeout = None;
                Some(Return::Update(self.state.clone()))
            }
            None => {
                if self.check_proposal_timeout(cx) {
                    log::debug!(round = self.state.current_round, "Proposal timed out",);
                    self.timeout = None;
                    Some(Return::Update(self.state.clone()))
                } else {
                    None
                }
            }
        }
    }

    /// Evaluates a given `proposal_hash` with corresponding VR, to check whether or not it is voted for.
    ///
    /// This cannot fail and will always progress the step to Prevote.
    fn received_proposal_with_vr(&mut self, proposal_hash: TProtocol::ProposalHash, vr: u32) {
        log::debug!(
            round = self.state.current_round,
            vr,
            ?proposal_hash,
            "Proposal with VR received",
        );

        // Check if a vote for the proposal can be cast.
        // For that the VR must be in the past.
        // The instance must not be locked, or it must be able to unlock.
        // The proposal must have gotten 2f+1 votes in the Prevote aggregation of round `vr`.
        if vr < self.state.current_round
            && self.is_allowed_to_vote_on(&proposal_hash, Some(vr))
            && self.has_two_f_plus_one(&proposal_hash, vr)
        {
            // This does not yet do anything to `self.state.locked`.
            self.state.votes.insert(
                (self.state.current_round, Step::Prevote),
                Some(proposal_hash),
            );
        } else {
            self.state
                .votes
                .insert((self.state.current_round, Step::Prevote), None);
        }

        // A proposal was received. Reset timeout, advance step and yield state.
        self.state.current_step = Step::Prevote;
    }

    /// Evaluates a given `proposal_hash`, to check whether or not it is voted for.
    ///
    /// This cannot fail and will always progress the step to Prevote.
    fn received_proposal_without_vr(&mut self, proposal_hash: TProtocol::ProposalHash) {
        log::debug!(
            round = self.state.current_round,
            ?proposal_hash,
            "Proposal without VR received",
        );
        // Check if allowed to vote for the proposal.
        if self.is_allowed_to_vote_on(&proposal_hash, None) {
            // Vote for the proposal if allowed to vote for the proposal.
            self.state.votes.insert(
                (self.state.current_round, Step::Prevote),
                Some(proposal_hash),
            );
        } else {
            // Vote None if not allowed to vote on the proposal for any reason.
            self.state
                .votes
                .insert((self.state.current_round, Step::Prevote), None);
        }

        // A proposal was received. Reset timeout, advance step and yield state.
        self.state.current_step = Step::Prevote;
    }

    /// In case no proposal was received when await proposal is called, the timeout must be checked.
    /// Once the timeout expires a vote for None must be added to the state advancing the step to Prevote.
    /// That state must be yielded.
    fn check_proposal_timeout(&mut self, cx: &mut Context<'_>) -> bool {
        // Check if the timeout elapsed.
        if self.timeout.as_mut().unwrap().poll_unpin(cx).is_ready() {
            // The timeout elapsed, vote nil as the proposal did not arrive in time.
            self.state
                .votes
                .insert((self.state.current_round, Step::Prevote), None);
            self.state.current_step = Step::Prevote;
            true
        } else {
            // Timeout not elapsed yet. Return None, to keep waiting.
            false
        }
    }
}
