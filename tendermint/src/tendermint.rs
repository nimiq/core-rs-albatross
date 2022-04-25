use std::{collections::BTreeMap, task::Poll};

use futures::{future::BoxFuture, FutureExt, Stream};

use crate::{
    aggregation_to_vote, has_2f1_votes,
    outside_deps::TendermintOutsideDeps,
    state::{Aggregate, ExtendedProposal, TendermintState},
    utils::{Step, TendermintReturn},
    AggregationResult, Checkpoint, ProposalResult, TendermintError, VoteResult,
};

// to not store each future individually even though there can only ever be one.
pub enum OutsideDepsFutReturn<DepsTy: TendermintOutsideDeps> {
    AwaitProposal(
        (
            DepsTy,
            ProposalResult<DepsTy::ProposalTy, DepsTy::ProposalCacheTy>,
        ),
    ),
    BroadcastProposal(DepsTy),
    Aggregate(
        (
            DepsTy,
            AggregationResult<DepsTy::ProposalHashTy, DepsTy::ProofTy>,
        ),
    ),
}

/// This is the struct that implements the Tendermint state machine. Its only fields are deps
/// (dependencies, any type that implements the trait TendermintOutsideDeps, needed for a variety of
/// low-level tasks) and state (stores the current state of Tendermint). The future option and the
/// TendermintOutSideDeps option are mutually exclusive.
pub struct Tendermint<DepsTy: TendermintOutsideDeps> {
    deps: Option<DepsTy>,
    state: TendermintState<
        DepsTy::ProposalTy,
        DepsTy::ProposalCacheTy,
        DepsTy::ProposalHashTy,
        DepsTy::ProofTy,
    >,

    fut: Option<BoxFuture<'static, Result<OutsideDepsFutReturn<DepsTy>, TendermintError>>>,
}

impl<DepsTy: TendermintOutsideDeps> Tendermint<DepsTy> {
    pub fn new(
        deps: DepsTy,
        state_opt: Option<
            TendermintState<
                DepsTy::ProposalTy,
                DepsTy::ProposalCacheTy,
                DepsTy::ProposalHashTy,
                DepsTy::ProofTy,
            >,
        >,
    ) -> Self {
        if let Some(state) = state_opt {
            Self {
                state,
                deps: Some(deps),
                fut: None,
            }
        } else {
            Self {
                state: TendermintState::new(deps.block_height(), deps.initial_round()),
                deps: Some(deps),
                fut: None,
            }
        }
    }

    fn start_round(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        // In this state a future is never created, thus the deps must exist at all times.
        // pretty much everything is none as this is the start of the protocol and nothing is known yet.
        // Neither a proposal nor the vote this node is going to make must exist
        assert!(self.deps.is_some());
        assert!(self.state.is_our_turn.is_none());
        assert!(self.state.current_proposal.is_none());
        assert!(self.state.current_proposal_vr.is_none());
        assert!(self.state.current_proof.is_none());
        assert!(!self.state.known_proposals.contains_key(&self.state.round));
        assert!(!self
            .state
            .own_votes
            .contains_key(&(self.state.round, self.state.step)));
        assert_eq!(self.state.step, Step::Propose);

        let mut deps = self.deps.take().unwrap();

        // Check if it is our turn to propose a value
        let round = self.state.round;
        if deps.is_our_turn(round) {
            // create and store proposal in case no valid round exists. Otherwise take the valid value.
            self.state.current_proposal = if let Some(valid) = self.state.valid.as_ref() {
                let valid_round = valid.round;

                debug!(
                    "Our turn at round {}, broadcasting former valid proposal of round {}",
                    &round, &valid_round,
                );

                self.state
                    .known_proposals
                    .insert(round, (valid.proposal.clone(), Some(valid_round)));
                self.state.current_proposal_vr = Some(valid_round);
                self.state.proposal_cache = self.state.vr_proposal_cache.clone();
                self.state
                    .own_votes
                    .insert((round, Step::Prevote), Some(valid.proposal_hash.clone()));
                let current_proposal = (valid.proposal.clone(), valid.proposal_hash.clone());
                Some(Some(current_proposal))
            } else {
                debug!("Our turn at round {}, broadcasting fresh proposal", round);
                match deps.get_value(round) {
                    Err(err) => return Poll::Ready(Some(TendermintReturn::Error(err))),
                    Ok((proposal, proposal_cache)) => {
                        self.state
                            .known_proposals
                            .insert(round, (proposal.clone(), None));
                        let proposal_hash =
                            deps.hash_proposal(proposal.clone(), proposal_cache.clone());
                        self.state.proposal_cache = Some(proposal_cache);
                        self.state
                            .own_votes
                            .insert((round, Step::Prevote), Some(proposal_hash.clone()));
                        Some(Some((proposal, proposal_hash)))
                    }
                }
            };

            self.state.is_our_turn = Some(true);
            self.state.current_checkpoint = Checkpoint::Propose;
            self.deps = Some(deps);
            // yield state
        } else {
            self.state.is_our_turn = Some(false);
            self.state.current_checkpoint = Checkpoint::WaitForProposal;
            self.deps = Some(deps);
            // yield state
        }
        Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
    }

    fn propose(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Option<<Self as Stream>::Item>> {
        // Propose does only make sense when this node is the producer. A proposal must exist.
        // Since this node will vote for its own proposal the vote is also required
        assert!(self.deps.is_some() ^ self.fut.is_some());
        assert_eq!(self.state.is_our_turn, Some(true));
        assert!(self.state.current_proposal.is_some());
        assert!(self.state.current_proposal.as_ref().unwrap().is_some());
        assert!(self.state.current_proof.is_none());
        // redundant but still good to check
        assert!(self.state.known_proposals.contains_key(&self.state.round));
        assert!(self
            .state
            .own_votes
            .contains_key(&(self.state.round, Step::Prevote)));
        assert_eq!(self.state.step, Step::Propose);

        let mut fut = if let Some(deps) = self.deps.take() {
            let proposal = match &self.state.current_proposal {
                Some(Some((proposal, _))) => proposal.clone(),
                _ => unreachable!(),
            };
            deps.broadcast_proposal(self.state.round, proposal, self.state.current_proposal_vr)
                .map(|r| r.map(|r| OutsideDepsFutReturn::BroadcastProposal(r)))
                .boxed()
        } else {
            self.fut.take().unwrap()
        };

        match fut.poll_unpin(cx) {
            Poll::Ready(Ok(OutsideDepsFutReturn::BroadcastProposal(deps))) => {
                self.state.step = Step::Prevote;
                self.state.current_checkpoint = Checkpoint::AggregatePreVote;
                self.deps = Some(deps);
                Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
            }
            Poll::Ready(Ok(_)) => panic!("Wrong Future Return type"),
            Poll::Ready(Err(err)) => Poll::Ready(Some(TendermintReturn::Error(err))),
            Poll::Pending => {
                self.fut = Some(fut);
                Poll::Pending
            }
        }
    }

    fn await_proposal(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        assert!(self.deps.is_some() ^ self.fut.is_some());
        assert_eq!(self.state.is_our_turn, Some(false));
        assert!(self.state.current_proposal.is_none());
        assert!(self.state.current_proposal_vr.is_none());
        assert!(self.state.current_proof.is_none());
        assert!(!self.state.known_proposals.contains_key(&self.state.round));
        assert!(!self
            .state
            .own_votes
            .contains_key(&(self.state.round, self.state.step)));
        assert_eq!(self.state.step, Step::Propose);

        let mut fut = if let Some(deps) = self.deps.take() {
            deps.await_proposal(self.state.round)
                .map(|r| r.map(|r| OutsideDepsFutReturn::AwaitProposal(r)))
                .boxed()
        } else {
            self.fut.take().unwrap()
        };

        match fut.poll_unpin(cx) {
            Poll::Ready(Ok(OutsideDepsFutReturn::AwaitProposal((deps, ret)))) => {
                match ret {
                    ProposalResult::Proposal((proposal, proposal_cache), Some(valid_round)) => {
                        let round = self.state.round;
                        // if a node presents a valid_round alognside the proposal, we must verify it
                        // first some sanity checks
                        if valid_round >= deps.initial_round() && valid_round < round {
                            let proposal_hash =
                                deps.hash_proposal(proposal.clone(), proposal_cache.clone());
                            self.state.current_proposal_vr = Some(valid_round);
                            self.state.current_proposal =
                                Some(Some((proposal.clone(), proposal_hash)));
                            self.state.proposal_cache = Some(proposal_cache);
                            // we must check if the valid round referenced has 2f+1 votes for this proposal in the Prevote step
                            self.state.current_checkpoint = Checkpoint::VerifyValidRound;
                        } else {
                            // invalid, thus vote None
                            // retry for another proposal?
                            self.state.current_proposal = Some(None);
                            self.state.own_votes.insert((round, Step::Prevote), None);
                            self.state.step = Step::Prevote;
                            self.state.current_checkpoint = Checkpoint::AggregatePreVote;
                        }
                        self.state
                            .known_proposals
                            .insert(round, (proposal, Some(valid_round)));
                        self.deps = Some(deps);
                        // yield state
                        Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
                    }
                    ProposalResult::Proposal((proposal, proposal_cache), None) => {
                        // a proposal without a valid round can be checked immediately
                        let round = self.state.round;
                        if let Some(locked) = self.state.locked.as_ref() {
                            if locked.proposal == proposal {
                                self.state.current_proposal =
                                    Some(Some((proposal.clone(), locked.proposal_hash.clone())));
                                self.state.own_votes.insert(
                                    (round, Step::Prevote),
                                    Some(locked.proposal_hash.clone()),
                                );
                            } else {
                                let proposal_hash =
                                    deps.hash_proposal(proposal.clone(), proposal_cache.clone());
                                self.state.current_proposal =
                                    Some(Some((proposal.clone(), proposal_hash)));
                                self.state.own_votes.insert((round, Step::Prevote), None);
                            }
                        } else {
                            let proposal_hash =
                                deps.hash_proposal(proposal.clone(), proposal_cache.clone());
                            self.state.current_proposal =
                                Some(Some((proposal.clone(), proposal_hash.clone())));
                            self.state
                                .own_votes
                                .insert((round, Step::Prevote), Some(proposal_hash));
                        }
                        // The proposal cache must be updated regardless of vote, as this node might unlock itself.
                        self.state.proposal_cache = Some(proposal_cache);
                        self.state.known_proposals.insert(round, (proposal, None));
                        self.state.step = Step::Prevote;
                        self.state.current_checkpoint = Checkpoint::AggregatePreVote;
                        self.deps = Some(deps);
                        // yield state
                        Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
                    }
                    ProposalResult::Timeout => {
                        // Store given information
                        let round = self.state.round;
                        self.state.current_proposal = Some(None);
                        self.state.own_votes.insert((round, Step::Prevote), None);
                        self.state.step = Step::Prevote;
                        self.state.current_checkpoint = Checkpoint::AggregatePreVote;
                        self.deps = Some(deps);
                        // yield state
                        Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
                    }
                }
            }
            Poll::Ready(Ok(_)) => panic!("Wrong Future Return type"),
            Poll::Ready(Err(err)) => Poll::Ready(Some(TendermintReturn::Error(err))),
            Poll::Pending => {
                self.fut = Some(fut);
                Poll::Pending
            }
        }
    }

    fn verify_valid_round(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        assert!(self.deps.is_some() ^ self.fut.is_some());
        assert_eq!(self.state.is_our_turn, Some(false));
        assert!(self.state.current_proposal.is_some());
        assert!(self.state.current_proposal.as_ref().unwrap().is_some());
        assert!(self.state.current_proposal_vr.is_some());
        assert!(self.state.known_proposals.contains_key(&self.state.round));
        assert!(!self
            .state
            .own_votes
            .contains_key(&(self.state.round, self.state.step)));
        assert_eq!(self.state.step, Step::Propose);

        let mut fut = if let Some(deps) = self.deps.take() {
            deps.get_aggregation(self.state.current_proposal_vr.unwrap(), Step::Prevote)
                .map(|r| r.map(|r| OutsideDepsFutReturn::Aggregate(r)))
                .boxed()
        } else {
            self.fut.take().unwrap()
        };

        match fut.poll_unpin(cx) {
            Poll::Ready(Ok(OutsideDepsFutReturn::Aggregate((deps, aggregate)))) => {
                let round = self.state.round;
                let (proposal, proposal_hash) =
                    self.state.current_proposal.clone().unwrap().unwrap();

                if has_2f1_votes(proposal_hash.clone(), aggregate) {
                    // check the locked state of this node
                    if let Some(locked) = self.state.locked.as_ref() {
                        // If it is locked, make sure it is allowed to vote for this
                        let vote_block = locked.proposal == proposal
                            || locked.round <= self.state.current_proposal_vr.unwrap();
                        if vote_block {
                            self.state
                                .own_votes
                                .insert((round, Step::Prevote), Some(proposal_hash));
                        } else {
                            // otherwise vote Nil
                            // retry for another proposal?
                            self.state.own_votes.insert((round, Step::Prevote), None);
                        }
                    } else {
                        // not locked, vote for the block
                        self.state
                            .own_votes
                            .insert((round, Step::Prevote), Some(proposal_hash));
                    }
                } else {
                    // proposal does not fullfill prior aggregation requirement, reject
                    self.state.own_votes.insert((round, Step::Prevote), None);
                }
                self.state.step = Step::Prevote;
                self.state.current_checkpoint = Checkpoint::AggregatePreVote;
                self.deps = Some(deps);
                // yield state
                Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
            }
            Poll::Ready(Ok(_)) => panic!("Wrong Future Return type"),
            Poll::Ready(Err(err)) => Poll::Ready(Some(TendermintReturn::Error(err))),
            Poll::Pending => {
                self.fut = Some(fut);
                Poll::Pending
            }
        }
    }

    fn aggregate(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        assert!(self.state.step == Step::Prevote || self.state.step == Step::Precommit);
        assert!(self.deps.is_some() ^ self.fut.is_some());
        assert!(self.state.is_our_turn.is_some());
        assert!(self.state.current_proposal.is_some());
        assert!(self
            .state
            .own_votes
            .contains_key(&(self.state.round, Step::Prevote)));
        assert!(
            self.state.step == Step::Prevote
                || self
                    .state
                    .own_votes
                    .contains_key(&(self.state.round, Step::Precommit))
        );

        let mut fut = if let Some(deps) = self.deps.take() {
            let vote = self
                .state
                .own_votes
                .get(&(self.state.round, self.state.step))
                .unwrap()
                .clone();
            deps.broadcast_and_aggregate(self.state.round, self.state.step, vote)
                .map(|r| r.map(|r| OutsideDepsFutReturn::Aggregate(r)))
                .boxed()
        } else {
            self.fut.take().unwrap()
        };

        match fut.poll_unpin(cx) {
            Poll::Ready(Ok(OutsideDepsFutReturn::Aggregate((
                deps,
                AggregationResult::Aggregation(aggregate),
            )))) => {
                let current_proposal_hash = self
                    .state
                    .current_proposal
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .map(|(_, hash)| hash.clone());

                let id = (self.state.round, self.state.step);
                // assume this is the best vote and store it in the state.
                // TODO update this with each aggregation step
                self.state.best_votes.insert(
                    id,
                    Aggregate {
                        aggregate: aggregate.clone(),
                    },
                );

                match self.state.step {
                    Step::Prevote => {
                        self.process_prevote_aggregation(deps, current_proposal_hash, aggregate)
                    }
                    Step::Precommit => {
                        self.process_precommit_aggregation(deps, current_proposal_hash, aggregate)
                    }
                    _ => panic!(
                        "Tendermint::aggregate is not in either Step::Prevote or Step::Precommit"
                    ),
                }
            }
            Poll::Ready(Ok(OutsideDepsFutReturn::Aggregate((
                deps,
                AggregationResult::NewRound(new_round),
            )))) => {
                if new_round > self.state.round {
                    // prepare state to start a new round
                    self.state.round = new_round;
                    self.state.is_our_turn = None;
                    self.state.current_proposal = None;
                    self.state.current_proposal_vr = None;
                    self.state.current_proof = None;
                    self.state.step = Step::Propose;
                    self.state.current_checkpoint = Checkpoint::StartRound;
                    self.deps = Some(deps);
                } else {
                    log::error!(
                        "Received outdated NewRound({}) event while in round {}-{:?}",
                        new_round,
                        self.state.round,
                        self.state.step,
                    );
                }
                // yield state
                Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
            }
            Poll::Ready(Ok(_)) => panic!("Wrong Future Return type"),
            Poll::Ready(Err(err)) => Poll::Ready(Some(TendermintReturn::Error(err))),
            Poll::Pending => {
                self.fut = Some(fut);
                Poll::Pending
            }
        }
    }

    fn process_prevote_aggregation(
        &mut self,
        deps: DepsTy,
        current_proposal_hash: Option<DepsTy::ProposalHashTy>,
        aggregate: BTreeMap<
            Option<<DepsTy as TendermintOutsideDeps>::ProposalHashTy>,
            (<DepsTy as TendermintOutsideDeps>::ProofTy, u16),
        >,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        // this will only result in a Block vote if we know the proposal, and have 2f+1 Precommits.
        match aggregation_to_vote::<DepsTy::ProposalHashTy, DepsTy::ProofTy>(
            &current_proposal_hash,
            aggregate,
        ) {
            VoteResult::Block(_) => {
                let round = self.state.round;
                if let Some(Some((p, h))) = &self.state.current_proposal {
                    let extended_proposal = ExtendedProposal {
                        round: self.state.round,
                        proposal: p.clone(),
                        proposal_hash: h.clone(),
                    };
                    // This node must lock itself on the proposal as it is going to precommit on it
                    self.state.locked = Some(extended_proposal.clone());
                    self.state.valid = Some(extended_proposal);
                    // additioally keep track of the proposal_cached for the VR
                    self.state.vr_proposal_cache = self.state.proposal_cache.clone();
                    // TODO: Once intermediate aggregation result are received from all aggregations
                    // the self.state.valid field can potentially be updated when receiving 2f+1 prevotes
                    // even though the node has already voted Nil for the precommit.
                    // The protocol says to do this only for the active round, but progressing the
                    // self.state.valid.round is likely always possible
                    // Since valid is a changes every time locked changes (but not vice-versa) they are
                    // identical unntil this change.
                }

                self.state
                    .own_votes
                    .insert((round, Step::Precommit), current_proposal_hash);
            }
            VoteResult::Nil(_) | VoteResult::Timeout => {
                let round = self.state.round;
                self.state.own_votes.insert((round, Step::Precommit), None);
            }
        };
        self.state.step = Step::Precommit;
        self.state.current_checkpoint = Checkpoint::AggregatePreCommit;
        self.deps = Some(deps);
        // yield state
        Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
    }

    fn process_precommit_aggregation(
        &mut self,
        deps: DepsTy,
        current_proposal_hash: Option<DepsTy::ProposalHashTy>,
        aggregate: BTreeMap<
            Option<<DepsTy as TendermintOutsideDeps>::ProposalHashTy>,
            (<DepsTy as TendermintOutsideDeps>::ProofTy, u16),
        >,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        match aggregation_to_vote::<DepsTy::ProposalHashTy, DepsTy::ProofTy>(
            &current_proposal_hash,
            aggregate,
        ) {
            VoteResult::Block(proof) => {
                log::debug!(
                    "Voting succeeded in round {}, assembling block.",
                    self.state.round
                );
                // this will terminate tendermint stream on the next poll.
                // Currently the SelectAll in stream.rs will drop the tendermint stream but keep
                // the background task. The background task is likely going to be removed thus for now
                // the propagation of Poll::Ready(None) was not implemented.
                self.state.current_proof = Some(proof.clone());

                // if we voted for this proposal we can assemble the block.
                // Otherwise we do not know it and thus can not produce the block.
                // aggregation_to_vote will only return VoteResult::Block if the node
                // knows the proposal.
                Poll::Ready(Some(
                    match deps.assemble_block(
                        self.state.round,
                        self.state.current_proposal.take().unwrap().unwrap().0,
                        self.state.proposal_cache.take(),
                        proof,
                    ) {
                        Ok(block) => TendermintReturn::Result(block),
                        Err(err) => TendermintReturn::Error(err),
                    },
                ))
            }
            VoteResult::Nil(_) | VoteResult::Timeout => {
                // prepare state to start the next round
                self.state.round += 1;
                self.state.is_our_turn = None;
                self.state.current_proposal = None;
                self.state.current_proposal_vr = None;
                self.state.current_proof = None;
                self.state.step = Step::Propose;
                self.state.current_checkpoint = Checkpoint::StartRound;
                self.deps = Some(deps);
                // yield state
                Poll::Ready(Some(TendermintReturn::StateUpdate(self.state.clone())))
            }
        }
    }
}

impl<DepsTy: TendermintOutsideDeps> Stream for Tendermint<DepsTy> {
    type Item = TendermintReturn<DepsTy>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.state.current_proof.is_some() {
            return Poll::Ready(None);
        }
        // State machine implementation
        match self.state.current_checkpoint {
            Checkpoint::StartRound => self.start_round(cx),
            Checkpoint::Propose => self.propose(cx),
            Checkpoint::WaitForProposal => self.await_proposal(cx),
            Checkpoint::VerifyValidRound => self.verify_valid_round(cx),
            Checkpoint::AggregatePreVote | Checkpoint::AggregatePreCommit => self.aggregate(cx),
        }
    }
}
