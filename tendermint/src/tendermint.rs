use std::{
    collections::{
        btree_map::{BTreeMap, Entry},
        BTreeSet,
    },
    pin::Pin,
    task::{Context, Poll, Waker},
};

use futures::{
    future::{BoxFuture, FutureExt},
    stream::{BoxStream, FuturesUnordered, SelectAll, Stream, StreamExt},
};
use nimiq_collections::BitSet;
use nimiq_macros::store_waker;
use tokio::{
    sync::mpsc,
    time::{error::Elapsed, Duration},
};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    protocol::{Aggregation, Protocol, SignedProposalMessage, TaggedAggregationMessage},
    state::State,
    utils::{Return, Step},
    AggregationMessage, Proposal,
};

/// Main Tendermint structure.
///
/// Implements `Stream<Item = Return<TProtocol>>`.
pub struct Tendermint<TProtocol: Protocol> {
    /// Protocol defining interactions with the caller side of Tendermint.
    pub(crate) protocol: TProtocol,

    /// The current state of this instance. A clone of it will be returned as the stream item when changes occur.
    pub(crate) state: State<TProtocol>,

    /// Stream containing all received proposals, already filtered for the current height.
    proposal_stream: BoxStream<
        'static,
        SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>,
    >,

    /// Collection of Futures created by requesting proposals from other nodes
    requested_proposals: FuturesUnordered<
        BoxFuture<
            'static,
            (
                Option<SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>>,
                (TProtocol::ProposalHash, u32),
            ),
        >,
    >,

    /// Pending requests for proposals
    pending_proposal_requests: BTreeSet<(TProtocol::ProposalHash, u32)>,

    /// Stream of all received contribution messages for aggregations relating to this height.
    /// Contributions for other heights are already filtered out.
    level_update_stream:
        BoxStream<'static, TaggedAggregationMessage<TProtocol::AggregationMessage>>,

    /// SelectAll over all started aggregations.
    pub(crate) aggregations: SelectAll<BoxStream<'static, ((u32, Step), TProtocol::Aggregation)>>,

    /// Keeps track of contributions to future aggregations.
    pub(crate) future_contributions: BTreeMap<u32, BitSet>,

    /// Keeps at most one future aggregation per validator, while it is waiting for verification.
    future_round_messages: BTreeMap<u16, TaggedAggregationMessage<TProtocol::AggregationMessage>>,

    /// The future round aggregation that is currently being verified.
    future_round_verification: Option<BoxFuture<'static, Result<(u32, BitSet), ()>>>,

    /// In case a timeout is required it will be stored here until elapsed or no longer necessary.
    /// Must be cleared in both cases.
    pub(crate) timeout: Option<BoxFuture<'static, Result<(), Elapsed>>>,

    /// Keeps track of a state return that still needs to happen. Whenever a proposal is Rejected/Ignored/Accepted
    /// it will be returned as a stream item but the state change following it must be returned on the next call
    /// of the poll function as well.
    state_return_pending: bool,

    /// Set once a decision was reached. The next call to poll will then return Poll::Ready(None)
    decision: bool,

    /// Used to dispatch messages to aggregations.
    pub(crate) aggregation_senders:
        BTreeMap<(u32, Step), mpsc::Sender<TProtocol::AggregationMessage>>,

    /// Waker used for the poll next function
    pub(crate) waker: Option<Waker>,
}

impl<TProtocol: Protocol> Tendermint<TProtocol> {
    pub fn new(
        dependencies: TProtocol,
        state_opt: Option<State<TProtocol>>,
        proposal_stream: BoxStream<
            'static,
            SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>,
        >,
        level_update_stream: BoxStream<
            'static,
            TaggedAggregationMessage<TProtocol::AggregationMessage>,
        >,
    ) -> Self {
        let mut this = Self {
            protocol: dependencies,
            state: state_opt.unwrap_or_default(),
            proposal_stream,
            requested_proposals: FuturesUnordered::default(),
            pending_proposal_requests: BTreeSet::default(),
            level_update_stream,
            aggregations: SelectAll::default(),
            aggregation_senders: BTreeMap::default(),
            future_contributions: BTreeMap::default(),
            future_round_messages: BTreeMap::default(),
            future_round_verification: None,
            timeout: None,
            decision: false,
            state_return_pending: false,
            waker: None,
        };

        this.init();

        this
    }

    /// Initializes the instance. In particular restarts aggregations if the initial state is not default().
    /// The currently ongoing round will not be started, as the first poll will do that.
    fn init(&mut self) {
        for ((round, step), vote) in self.state.votes.iter() {
            // Start every aggregation except for the current one, as it is started by the state machine implementation
            if self.state.current_round == *round && self.state.current_step == *step {
                continue;
            }

            let id = (*round, *step);
            if let Entry::Vacant(entry) = self.aggregation_senders.entry(id) {
                log::debug!(round, ?step, "Restarting aggregation.",);

                let (sender, receiver) = mpsc::channel(100);
                // create the aggregation
                let aggregation = self.protocol.create_aggregation(
                    *round,
                    *step,
                    vote.clone(),
                    ReceiverStream::new(receiver).boxed(),
                );
                // Store the sender for the aggregation
                entry.insert(sender);
                // add the aggregation to the SelectAll of all aggregations
                self.aggregations
                    .push(aggregation.map(move |item| (id, item)).boxed());
            }
        }
    }

    /// Starts a timeout future for the current round if none exists yet.
    ///
    /// Has no effect if the timeout future already exists.
    pub(crate) fn start_timeout(&mut self) {
        if self.timeout.is_none() {
            let duration = Duration::from_millis(
                TProtocol::TIMEOUT_INIT
                    + self.state.current_round as u64 * TProtocol::TIMEOUT_DELTA,
            );
            self.timeout = Some(tokio::time::timeout(duration, futures::future::pending()).boxed());
        }
    }

    /// Test `proposal_hash` against the locked value in state using an optional `vr`.
    ///
    /// Returns false if locked is set and the hash is *not* equal to `proposal_hash` and `vr` is not
    /// more recent than the round this instance is locked on.
    ///
    /// Returns true otherwise.
    pub(crate) fn is_allowed_to_vote_on(
        &self,
        proposal_hash: &TProtocol::ProposalHash,
        vr: Option<u32>,
    ) -> bool {
        match &self.state.locked {
            // Always allowed to vote when not locked.
            None => true,
            // If locked, only allowed to vote for it if it is the same hash, or vr is more recent.
            Some((locked_round, locked_hash)) => {
                locked_hash == proposal_hash || Some(*locked_round) < vr
            }
        }
    }

    /// Checks if an aggregation exists for `round` and `Step::Prevote` where `proposal_hash` got at least 2f+1 votes.
    pub(crate) fn has_two_f_plus_one(
        &self,
        proposal_hash: &TProtocol::ProposalHash,
        round: u32,
    ) -> bool {
        self.state
            .best_votes
            .get(&(round, Step::Prevote))
            .and_then(|aggregate| {
                if aggregate.contributors_for(Some(proposal_hash)).len()
                    >= TProtocol::TWO_F_PLUS_ONE
                {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
    }

    /// Processes gossipped proposals. Only a single gossipped proposal will be accepted for each round.
    /// Subsequent proposals for a round will be ignored and returned on the stream as such.
    ///
    /// In case the state is altered, which happens in case Some(Return::ProposalAccepted(...)) is returned an additional
    /// flag is set such that  the next call to poll will return the state even when it would return Poll::Pending otherwise.
    ///
    /// Returns
    /// * `None` when no proposals was received at all.
    /// * `Some(Return::ProposalAccepted(...))` in case the proposal was the first for its round and it was valid.
    /// * `Some(Return::ProposalRejected(...))` in case the proposal was the first for its round and it was not valid.
    /// * `Some(Return::ProposalIgnored(...))` in case the proposal was not the first for its round.
    fn process_next_proposal(&mut self, cx: &mut Context<'_>) -> Option<Return<TProtocol>> {
        if let Poll::Ready(Some(proposal)) = self.proposal_stream.poll_next_unpin(cx) {
            let proposal_round = proposal.message.round;

            // Only at most 1 proposal from gossip is of interest.
            // Other interesting proposals will be requested separately.
            // This will also create the entry if it did not exist yet.
            //
            // Small improvement: Keep the inner `&mut BTreeMap<_>` around, as it can be used within self.process_proposal
            //      to add the proposal if verification is successful
            if !self
                .state
                .round_proposals
                .entry(proposal_round)
                .or_default()
                .is_empty()
            {
                // Already got a proposal for this round and step, thus
                // return the proposal as ignored and do nothing further with it.
                return Some(Return::ProposalIgnored(proposal));
            }

            if self.process_proposal(proposal.clone()).is_some() {
                self.state_return_pending = true;

                return Some(Return::ProposalAccepted(proposal));
            } else {
                return Some(Return::ProposalRejected(proposal));
            }
        }

        // Nothing to process, return None.
        None
    }

    /// Processes a signed proposal received by any means.
    ///
    /// Returns Some(proposal_hash) in case the proposal was added to the state, None otherwise.
    ///
    /// Failing to verify:
    /// * As failing to verify could indicate a malicious validator we might want to introduce punishment here. One
    /// option to do so is, similar to how we return proposals on the stream, to introduce another `Return` enum variant for that.
    /// * Potentially make a difference between incorrect signatures and incorrect proposals when it comes to punishment.
    fn process_proposal(
        &mut self,
        proposal: SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>,
    ) -> Option<TProtocol::ProposalHash> {
        let inherent_entry = self
            .state
            .inherents
            .entry(proposal.message.proposal.inherent_hash());

        // check if the proposals inherent is already known.
        let proposal_hash = match inherent_entry {
            Entry::Vacant(inherent_entry) => {
                // The inherent is unknown. The proposal itself cannot be known either.
                // Do full verification while also creating the inherent.
                match self.protocol.verify_proposal(&proposal, None, false) {
                    Ok(inherent) => {
                        // Use proposal and created inherent to produce the hash
                        let proposal_hash = proposal.message.proposal.hash();

                        // Cache the inherent
                        inherent_entry.insert(inherent);

                        // Failing here would indicate that even though the inherent was not cached the proposal was.
                        assert!(self
                            .state
                            .known_proposals
                            .insert(proposal_hash.clone(), proposal.message.proposal.clone())
                            .is_none());

                        // Return the hash such that it can be added to the round_proposals collection.
                        proposal_hash
                    }
                    Err(error) => {
                        // The proposal failed to verify.
                        log::debug!(
                            ?error,
                            ?proposal.message,
                            "Proposal did not verify"
                        );
                        return None;
                    }
                }
            }
            Entry::Occupied(inherent_entry) => {
                // The inherent is known. Use proposal and inherent to create the hash.
                let proposal_hash = proposal.message.proposal.hash();

                // Using the hash, retrieve the proposal, if it is already known.
                if let Entry::Vacant(proposal_entry) =
                    self.state.known_proposals.entry(proposal_hash.clone())
                {
                    // The proposal is not known, but the inherent is. Simplify verification as the inherent does not need to be produced.
                    match self.protocol.verify_proposal(
                        &proposal,
                        Some(inherent_entry.get().clone()),
                        false,
                    ) {
                        Ok(_) => {
                            // The proposal is valid.
                            // Insert it to the so far vacant entry in the known proposals collection and return the hash.
                            proposal_entry.insert(proposal.message.proposal.clone());
                            proposal_hash
                        }
                        Err(error) => {
                            // The proposal failed to verify.
                            log::debug!(
                                ?error,
                                ?proposal.message,
                                "Proposal did not verify"
                            );
                            return None;
                        }
                    }
                } else {
                    // The proposal as well as the inherent are known.
                    // Only verify the signature as the validity of the proposal itself has already been performed
                    // when it was originally added to `known_proposals`.
                    match self.protocol.verify_proposal(
                        &proposal,
                        Some(inherent_entry.get().clone()),
                        true,
                    ) {
                        Ok(_) => proposal_hash,
                        Err(error) => {
                            log::debug!(
                                ?error,
                                ?proposal.message,
                                "Proposal did not verify"
                            );
                            return None;
                        }
                    }
                }
            }
        };

        // The proposal with `proposal_hash` was successfully verified. Add it to the set of proposals for its round.
        self.state
            .round_proposals
            .entry(proposal.message.round)
            .or_default()
            .insert(
                proposal_hash.clone(),
                (proposal.message.valid_round, proposal.signature),
            );

        // make sure the next poll will return a state update
        Some(proposal_hash)
    }

    /// Dispatches incoming aggregation messages to the appropriate aggregations. If messages for future aggregations are received,
    /// they are accumulated and once any round reaches f+1 contributions that round is fast tracked to.
    ///
    /// Returns
    /// * `None` if all messages were successfully dispatched while `level_update_stream` was polled
    /// until it returned Poll::Pending.
    /// * `Some(tagged_message)` if any message exists for which the sender reports a full channel.
    /// In this case the stream was not polled until it returned Pending.
    fn dispatch_messages(
        &mut self,
        cx: &mut Context<'_>,
        should_export_state: &mut bool,
    ) -> Option<TaggedAggregationMessage<TProtocol::AggregationMessage>> {
        // Has any of the polled futures done something? If so, don't return. If we're out of
        // futures to poll, return.
        let mut did_something = true;
        while did_something {
            did_something = false;

            // First, check if the future round aggregation verification has completed.
            if let Some(verification) = &mut self.future_round_verification {
                if let Poll::Ready(verification_result) = verification.poll_unpin(cx) {
                    did_something = true;
                    self.future_round_verification = None;

                    // If the signature was found to be valid, add the contributing validators to
                    // the set of contributors for that future round.
                    if let Ok((round, new_contributors)) = verification_result {
                        if round > self.state.current_round {
                            // future contributions need to be tracked separately, as once a round has f+1 contributions
                            // it can be fast tracked
                            let contributors = self.future_contributions.entry(round).or_default();
                            *contributors |= new_contributors;

                            // check if the skip ahead condition is fulfilled. If so, the state machine can be set to propose for round id.0
                            if contributors.len() >= TProtocol::F_PLUS_ONE {
                                // skip to that round
                                *should_export_state = true;
                                self.state.current_round = round;
                                self.state.current_step = Step::Propose;
                                self.future_contributions.retain(|&round, _contributors| {
                                    round > self.state.current_round
                                });
                                self.future_round_messages
                                    .retain(|_, message| message.tag.0 > self.state.current_round);

                                // Return to account the state change
                                return None;
                            }
                        }
                    }
                }
            }

            // If no future round aggregation verification is currently running, start a new one.
            if self.future_round_verification.is_none() && !self.future_round_messages.is_empty() {
                // TODO: choose at random
                let (_, message) = self.future_round_messages.pop_first().unwrap();
                let (round, step) = message.tag;
                let contributors = message.aggregation.all_contributors();
                let verification = self
                    .protocol
                    .verify_aggregation_message(round, step, message.aggregation)
                    .map(move |result| result.map(move |()| (round, contributors)));
                self.future_round_verification = Some(Box::pin(verification));
            }

            // Finally, check if we have new messages.
            //
            // the state changes if there is at least one message to process as they are kept for future reference.
            if let Poll::Ready(Some(mut message)) = self.level_update_stream.poll_next_unpin(cx) {
                did_something = true;
                let id = message.tag;
                if id.0 > self.state.current_round {
                    // Queue the message for future round aggregation verification.
                    match self
                        .future_round_messages
                        .entry(message.aggregation.sender())
                    {
                        Entry::Vacant(v) => {
                            v.insert(message);
                        }
                        Entry::Occupied(mut o) => {
                            // Generally: Newer is better. If the new message comes
                            // from the same round as a previous one, assume that
                            // it is better.
                            if id.0 >= o.get().tag.0 {
                                o.insert(message);
                            }
                        }
                    }
                } else if let Some(sender) = self.aggregation_senders.get(&id) {
                    match sender.try_send(message.aggregation) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Closed(_aggregation_message)) => {
                            panic!("Aggregations must not be dropped.");
                        }
                        Err(mpsc::error::TrySendError::Full(aggregation)) => {
                            message.aggregation = aggregation;
                            // failed to send the message, which means a channel must be full.
                            // return the message, so after polling the aggregations it can be try_send again.
                            // that time it should succeed as the aggregation poll should have cleared the channels.
                            return Some(message);
                        }
                    }
                } else {
                    log::debug!(
                        ?message,
                        "received level update for non running aggregation",
                    );
                }
            }
        }
        None
    }

    /// Poll all ongoing aggregations updating the best_votes in the state.
    ///
    /// `should_export_state` will be set to true in case a value in the state improves. It is maintained to indicate whether
    /// a state export should happen or not.
    ///
    /// Returns
    /// * `Some(Decision)` if it was reached
    /// * `None` otherwise
    fn poll_aggregations(
        &mut self,
        cx: &mut Context<'_>,
        should_export_state: &mut bool,
    ) -> Option<TProtocol::Decision> {
        // Poll all aggregations until none of them returns an update
        while let Poll::Ready(Some((round_and_step, aggregate))) =
            self.aggregations.poll_next_unpin(cx)
        {
            // For every update received, first update the best_vote for that aggregation.
            // If the updated value is better (more votes) make sure, that a state update will be required.
            // If this is the first update for the aggregation, create the entry.
            let best_vote = self
                .state
                .best_votes
                .entry(round_and_step)
                .and_modify(|agg| {
                    if agg.all_contributors().len() < aggregate.all_contributors().len() {
                        *agg = aggregate.clone();
                        *should_export_state = true;
                    }
                })
                .or_insert_with(|| {
                    *should_export_state = true;
                    aggregate
                });

            // Get the total weight for the aggregate. It will be used for more comparisons later.
            let total_contributor_count = best_vote.all_contributors().len();

            for (proposal_hash, contributor_count) in best_vote.proposals() {
                // First check for the aggregate yielding a result like a decision, or a precommit block vote.
                if contributor_count >= TProtocol::TWO_F_PLUS_ONE {
                    if let Some(proposal) = self.state.known_proposals.get(&proposal_hash) {
                        match round_and_step.1 {
                            Step::Prevote => {
                                // The proposal exists and is valid. 2f+1 prevotes have been seen.
                                // If it is an improvement over `valid`, replace it.
                                // Note that this might happen for any given round. This instance might have already voted None.
                                // Thus setting `valid` and setting `locked` happens in different places.
                                if self.state.valid.is_none()
                                    || self.state.valid.as_ref().unwrap().0 < round_and_step.0
                                {
                                    self.state.valid =
                                        Some((round_and_step.0, proposal_hash.clone()));
                                }
                            }
                            Step::Precommit => {
                                // The proposal exists and is valid. 2f+1 precommits have been seen.
                                log::debug!(?round_and_step, "Aggregation produced decision value",);

                                // Get the inherent
                                let inherent = self
                                    .state
                                    .inherents
                                    .get(&proposal.inherent_hash())
                                    .expect("");

                                // Produce a decision
                                let decision = self.protocol.create_decision(
                                    proposal.clone(),
                                    inherent.clone(),
                                    best_vote.clone(),
                                    round_and_step.0,
                                );

                                return Some(decision);
                            }
                            _ => panic!("Aggregations must not have Step::Propose"),
                        };
                    } else {
                        // // Request proposal, as it is not known.
                        if !self
                            .pending_proposal_requests
                            .contains(&(proposal_hash.clone(), round_and_step.0))
                        {
                            log::debug!(
                                ?round_and_step,
                                ?best_vote,
                                "Unknown Proposal exceeds 2f+1 votes, without being requested.",
                            );
                        }
                    }
                }

                if contributor_count >= TProtocol::F_PLUS_ONE
                    && total_contributor_count - contributor_count < TProtocol::F_PLUS_ONE
                    && !self.state.known_proposals.contains_key(&proposal_hash)
                {
                    // Second check if the proposal has potential.
                    // Outside of the first proposal received via gossip that is valid, other proposals with potential would be
                    // if it has more than f votes while everything else has at most f votes.
                    // Reasoning is that once any valid proposal reaches 2f+1 prevotes this node needs to be able to vote on it.
                    // If that succeeds in time it will lock and valid round the proposal. If the request is not answered in
                    // time but received later it will only set the valid round if applicable.
                    // By making sure everything else has at most f votes there is enough voting power left for the proposal to reach 2f+1
                    log::debug!(
                        ?round_and_step,
                        ?best_vote,
                        "Aggregate contains proposal with potential. Requesting proposal",
                    );

                    let id = (proposal_hash.clone(), round_and_step.0);
                    if !self.pending_proposal_requests.contains(&id) {
                        self.pending_proposal_requests.insert(id.clone());
                        // Request the proposal, as it might be needed.
                        let response = self
                            .protocol
                            .request_proposal(
                                proposal_hash.clone(),
                                round_and_step.0,
                                best_vote.contributors_for(Some(&proposal_hash)),
                            )
                            .map(|response| (response, id))
                            .boxed();

                        // Add it to the list of pending responses
                        self.requested_proposals.push(response);

                        // Pushing the future to FuturesUnordered above does not wake the task that
                        // polls `requested_proposals`. Therefore, we need to wake the task manually.
                        if let Some(waker) = &self.waker {
                            waker.wake_by_ref();
                        }
                    }
                }
            }
        }
        None
    }
}

impl<TProtocol: Protocol> Stream for Tendermint<TProtocol> {
    type Item = Return<TProtocol>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

        // If a decision was returned previously this stream is terminated.
        if self.decision {
            return Poll::Ready(None);
        }

        // Poll gossipped proposals.
        let mut should_export_state = false;
        if let Some(proposal) = self.process_next_proposal(cx) {
            // In case a proposal was received it must be indicated if it was ignored, refused or accepted.
            // If the state has changed by processing the proposal the previous function call has already
            // set self.state_return_pending to true such that the subsequent poll call will return a state update as desired,
            // regardless of the actual result of the next poll call.
            return Poll::Ready(Some(proposal));
        }

        // These future might eventually return None once
        while let Poll::Ready(Some((signed_proposal, id))) =
            self.requested_proposals.poll_next_unpin(cx)
        {
            self.pending_proposal_requests.remove(&id);
            if let Some(signed_proposal) = signed_proposal {
                if self.process_proposal(signed_proposal).is_some() {
                    should_export_state = true;
                }
            }
            // Do nothing if the future ultimately resolved to None.
        }

        // Poll incoming level updates and dispatch to corresponding aggregations.
        let failed_message_opt = self.dispatch_messages(cx, &mut should_export_state);

        // Poll all currently ongoing aggregations
        if let Some(decision) = self.poll_aggregations(cx, &mut should_export_state) {
            self.decision = true;
            return Poll::Ready(Some(Return::Decision(decision)));
        };

        // If a message failed to dispatch, try again, as polling the aggregations should have cleared up the buffer.
        if let Some(message) = failed_message_opt {
            if let Some(sender) = self.aggregation_senders.get(&message.tag) {
                match sender.try_send(message.aggregation) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Closed(_aggregation_message)) => {
                        panic!("Channel was closed unexpectedly.");
                    }
                    Err(mpsc::error::TrySendError::Full(_aggregation_message)) => {
                        log::error!("Dispatching message failed with full channel twice!");
                    }
                }
            }
        }

        // Run the state machine implementation.
        let state_machine_return = match self.state.current_step {
            Step::Propose => {
                // Abort if we can't determine the proposer.
                let is_proposer = self.protocol.is_proposer(self.state.current_round);
                if is_proposer.is_err() {
                    // Make sure we only return None from now on.
                    self.decision = true;
                    return Poll::Ready(None);
                }

                if is_proposer.unwrap() {
                    // Abort if we can't create a proposal.
                    let state_machine_return = self.propose();
                    if state_machine_return.is_err() {
                        // Make sure we only return None from now on.
                        self.decision = true;
                        return Poll::Ready(None);
                    }
                    Some(state_machine_return.unwrap())
                } else {
                    self.await_proposal(cx)
                }
            }
            Step::Prevote | Step::Precommit => self.aggregate(cx),
        };

        // If the state machine did not return Poll::Pending its return must be returned as it might be the result
        if state_machine_return.is_some() {
            if let Some(Return::Update(_)) = &state_machine_return {
                self.state_return_pending = false;
            }
            return Poll::Ready(state_machine_return);
        }

        // If the state machine did return Poll::Pending there might have been a state change prior to it, that makes a state
        // export necessary.
        if state_machine_return.is_none() && (should_export_state || self.state_return_pending) {
            // Whenever returning the state the state_return_pending flag should be reset
            self.state_return_pending = false;
            return Poll::Ready(Some(Return::Update(self.state.clone())));
        }

        // The state machine returned Poll::Pending and so should the poll function. However whenever we have just
        // created an aggregation within running the state machine that aggregation has not been polled itself.
        // Thus it has not produced the first aggregate altering the state. If the poll function loops once to poll it,
        // it will always produce a first aggregate and thus a new meaningful state to return.

        if self.state.current_step != Step::Propose
            && !self
                .state
                .best_votes
                .contains_key(&(self.state.current_round, self.state.current_step))
            && self
                .aggregation_senders
                .contains_key(&(self.state.current_round, self.state.current_step))
        {
            // Poll all currently ongoing aggregations again, as a new aggregation was created.
            if let Some(decision) = self.poll_aggregations(cx, &mut should_export_state) {
                self.decision = true;
                return Poll::Ready(Some(Return::Decision(decision)));
            };

            if should_export_state {
                self.state_return_pending = false;
                return Poll::Ready(Some(Return::Update(self.state.clone())));
            }
        }

        // Return Poll::Pending in all other cases.
        Poll::Pending
    }
}
