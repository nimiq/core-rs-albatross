use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use futures::channel::mpsc::{unbounded as mpsc_channel, UnboundedReceiver, UnboundedSender};
use futures::{
    future, pin_mut, select,
    stream::{FusedStream, Stream, StreamExt},
    FutureExt,
};
use hash::{Blake2bHash, Hash};
use macros::upgrade_weak;
use network_interface::message::Message as NetworkInterfaceMessage;
use network_interface::network::Network as NetworkInterface;
use parking_lot::RwLock;
use std::clone::Clone;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use tokio::time::{delay_for, Duration};
use utils::mutable_once::MutableOnce;

pub struct Tendermint<
    Netw: NetworkInterface,
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
    ResultTy: Send + Sync + 'static,
    Deps: TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>,
> {
    deps: Deps,
    network: Netw,

    weak_self: MutableOnce<Weak<Self>>,
    state: Arc<RwLock<TendermintState<ProposalTy, ProofTy, ResultTy>>>,
}

struct TendermintState<ProposalTy, ProofTy, ResultTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
    ResultTy: Send + Sync + 'static,
{
    // we don't need height or decision
    round: u32,
    step: Step,
    locked_round: Option<(u32, Arc<ProposalTy>)>,
    valid_round: Option<(u32, Arc<ProposalTy>)>,

    return_stream: Option<UnboundedSender<TendermintReturn<ProposalTy, ProofTy, ResultTy>>>,

    current_proposal: Option<Arc<ProposalTy>>,
    current_proposal_valid_round: Option<u32>,
    current_proof: Option<ProofTy>,
}

pub struct TendermintStateProof<ProposalTy, ProofTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
{
    current_round: u32,
    current_step: Step,
    locked_round: Option<(u32, Arc<ProposalTy>)>,
    valid_round: Option<(u32, Arc<ProposalTy>)>,
    current_proposal: Option<Arc<ProposalTy>>,
    proof: Option<ProofTy>,
}

pub enum TendermintReturn<ProposalTy, ProofTy, ResultTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
    ProofTy: Clone + Send + Sync + 'static,
{
    Result(ResultTy),
    StateUpdate(TendermintStateProof<ProposalTy, ProofTy>),
    Error(TendermintBlockError),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Step {
    Propose,
    Prevote,
    Precommit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Lock<ProposalTy>
where
    ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static,
{
    None,
    Locked(u32, Arc<ProposalTy>),
}

pub enum TendermintBlockError {
    NetworkEnded,
    BadInitState,
}

// This Tendermint implementation tries to stick to the pseudocode of https://arxiv.org/abs/1807.04938v3
impl<
        Netw: NetworkInterface,
        ProposalTy: Clone
            + Eq
            + PartialEq
            + Serialize
            + Deserialize
            + Send
            + Sync
            + 'static
            + std::fmt::Display,
        ProofTy: Clone + Send + Sync + 'static,
        ResultTy: Send + Sync + 'static,
        DepsTy: Send
            + Sync
            + TendermintOutsideDeps<ProposalTy = ProposalTy, ResultTy = ResultTy, ProofTy = ProofTy>
            + 'static,
    > Tendermint<Netw, ProposalTy, ProofTy, ResultTy, DepsTy>
{
    pub fn new(
        deps: DepsTy,
        network: Netw,
    ) -> Arc<Tendermint<Netw, ProposalTy, ProofTy, ResultTy, DepsTy>> {
        let this = Arc::new(Self {
            deps,
            network,

            weak_self: MutableOnce::new(Weak::new()),
            state: Arc::new(RwLock::new(TendermintState {
                round: 0,
                step: Step::Propose,
                locked_round: None,
                valid_round: None,

                return_stream: None,

                current_proposal: None,
                current_proposal_valid_round: None,
                current_proof: None,
            })),
        });
        unsafe { this.weak_self.replace(Arc::downgrade(&this)) };
        this
    }

    pub fn expect_block(
        &self,
    ) -> UnboundedReceiver<TendermintReturn<ProposalTy, ProofTy, ResultTy>> {
        let (sender, receiver) = mpsc_channel();
        self.state.write().return_stream = Some(sender);

        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.start_round(1);
        });

        receiver
    }

    pub fn expect_block_from_state(
        &self,
        state: TendermintStateProof<ProposalTy, ProofTy>,
    ) -> UnboundedReceiver<TendermintReturn<ProposalTy, ProofTy, ResultTy>> {
        let (sender, receiver) = mpsc_channel();
        self.state.write().return_stream = Some(sender);

        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);

            let mut invalid_state = false;

            match state.current_step {
                Step::Propose => {
                    if state.proof.is_some()
                        && this
                            .deps
                            .verify_proposal_state(state.current_round, &state.proof.unwrap())
                    {
                        this.start_round(state.current_round);
                    } else {
                        invalid_state = true;
                    }
                }
                Step::Prevote => {
                    if state.current_proposal.is_some()
                        && this.deps.verify_prevote_state(
                            state.current_round,
                            state.current_proposal.unwrap(),
                            &state.proof.unwrap(),
                        )
                    {
                        this.broadcast_and_aggregate_prevote(
                            state.current_round,
                            SingleDecision::Block,
                        );
                    } else {
                        invalid_state = true;
                    }
                }
                Step::Precommit => {
                    if let Some(decision) = this.deps.verify_precommit_state(
                        state.current_round,
                        state.current_proposal,
                        &state.proof.unwrap(),
                    ) {
                        this.broadcast_and_aggregate_precommit(state.current_round, decision);
                    } else {
                        invalid_state = true;
                    }
                }
            };

            if invalid_state {
                if let Some(return_stream) = this.state.write().return_stream.as_mut() {
                    if let Err(_) = return_stream
                        .unbounded_send(TendermintReturn::Error(TendermintBlockError::BadInitState))
                    {
                        warn!("Failed sending/returning tendermint error")
                    }
                    return_stream.close_channel();
                }
            }
        });

        receiver
    }

    // Lines 11-21
    fn start_round(&self, round: u32) {
        self.state.write().round = round;
        self.state.write().step = Step::Propose;

        if self.deps.is_our_turn(round) {
            self.produce_send_proposal(round);
        } else {
            self.await_proposal(round);
        }
    }

    fn produce_send_proposal(&self, round: u32) {
        let mut state = self.state.write();
        let (proposal, valid_round) = if let Some((round, value)) = state.valid_round.clone() {
            (Some(value), round)
        } else {
            let new_proposal_opt = self.deps.produce_proposal(round);
            if let Some(new_proposal) = new_proposal_opt {
                let proposal_arc = Arc::new(new_proposal);
                state.current_proposal = Some(proposal_arc.clone());
                (Some(proposal_arc), round)
            } else {
                (None, round)
            }
        };
        drop(state);

        if let Some(p) = proposal {
            let proposal_msg = Message {
                proposal: p.clone(),
                round,
                valid_round,
            };
            let weak_self = self.weak_self.clone();
            tokio::spawn(async move {
                let this = upgrade_weak!(weak_self);
                this.network.broadcast(&proposal_msg).await;
            });

            // Prevote for our own proposal
            self.state.write().current_proposal_valid_round = Some(valid_round);
            self.state.write().step = Step::Prevote;
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Block);
        } else {
            warn!("Failed producing proposal in our round");
        }
    }

    fn await_proposal(&self, round: u32) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);

            let mut delay_duration = BLOCK_PROPOSAL_TIMEOUT;
            if let Some(additional_delay) = BLOCK_PROPOSAL_TIMEOUT_DELTA.checked_mul(round - 1) {
                if let Some(new_duration) = delay_duration.checked_add(additional_delay) {
                    delay_duration = new_duration;
                }
            }

            let mut stream = this.network.receive_from_all::<Message<ProposalTy>>();
            let f1 = stream.next().fuse();
            let f2 = delay_for(delay_duration).fuse();

            pin_mut!(f1, f2);

            loop {
                select!(
                    msg_opt = f1 => {
                        if let Some((msg, _)) = msg_opt {
                            if this.on_proposal(msg.proposal, msg.round, msg.valid_round) {
                                break;
                            }
                        } else {
                            // throw error
                            break;
                        }
                    },
                    () = f2 => {
                        // TODO also listen for proposals after timeout
                        this.on_timeout_propose(round);
                        break;
                    }
                );
            }
        });
    }

    // Lines  22-27
    fn on_proposal(&self, proposal: ProposalTy, round: u32, valid_round: u32) -> bool {
        let proposal_arc = Arc::new(proposal);

        let mut state = self.state.write();

        if round != state.round || state.step != Step::Propose {
            return false;
        }

        let mut should_prevote_block = false;

        if self.deps.verify_proposal(proposal_arc.clone(), round) {
            match &state.locked_round {
                None => {
                    should_prevote_block = true;
                }
                Some((locked_round, locked_value)) => {
                    if *locked_value == proposal_arc {
                        should_prevote_block = true;
                    }
                }
            }

            state.current_proposal = Some(proposal_arc);
            state.current_proposal_valid_round = Some(valid_round);
        }

        // self.return_state_update();

        state.step = Step::Prevote;

        if should_prevote_block {
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Block);
        } else {
            self.broadcast_and_aggregate_prevote(round, SingleDecision::Nil);
        }

        return true;
    }

    // Lines 28-33 are about unlocking. Done instead in `on_polka` and in `on_proposal`.

    // Lines 34-35: Handel takes care of waiting `timeoutPrevote` before returning

    // Lines 36-43
    fn on_polka(&self, round: u32) {
        let mut state = self.state.write();
        if state.current_proposal.is_none() || state.round != round || state.step == Step::Propose {
            return;
        }

        let current_proposal = if let Some(p) = &state.current_proposal {
            p.clone()
        } else {
            return;
        };

        let step = state.step;

        state.valid_round = Some((round, current_proposal.clone()));

        drop(state);

        if step == Step::Prevote {
            self.state.write().locked_round = Some((round, current_proposal));
            self.state.write().step = Step::Precommit;
            self.broadcast_and_aggregate_precommit(round, SingleDecision::Block);
        }
    }

    // Lines 44-46
    fn on_nil_polka(&self, round: u32) {
        let mut state = self.state.write();
        if !(round == state.round && state.step == Step::Prevote) {
            return;
        }

        state.step = Step::Precommit;
        drop(state);

        self.return_state_update();

        self.broadcast_and_aggregate_precommit(round, SingleDecision::Nil);
    }

    // Lines 47-48 Handel takes care of waiting `timeoutPrecommit` before returning

    // Lines 49-54
    fn on_2f1_block_precommits(&self, round: u32, proof: ProofTy) {
        let block_opt = if let Some(proposal) = self.state.read().current_proposal.clone() {
            Some(self.deps.assemble_block(proposal, proof))
        } else {
            // Might not have a proposal if block timeout -> prevote nil -> prevote timeout -> precommit nil -> here
            None // TODO wait for the proposal?
        };

        if let Some(block) = block_opt {
            if let Some(return_stream) = self.state.write().return_stream.as_mut() {
                if let Err(_) = return_stream.unbounded_send(TendermintReturn::Result(block)) {
                    warn!("Failed sending/returning completed macro block")
                }
                return_stream.close_channel();
            }
        }
    }

    // Lines 55-56 We instead trigger a validator state resync if we get too many unexpected level updates

    // Lines 57-60
    fn on_timeout_propose(&self, round: u32) {
        let mut state = self.state.write();

        if !(state.round == round && state.step == Step::Propose) {
            return;
        }
        state.step = Step::Prevote;

        drop(state);
        self.broadcast_and_aggregate_prevote(round, SingleDecision::Nil);
    }

    // Lines 61-64
    fn on_timeout_prevote(&self, round: u32) {
        let mut state = self.state.write();

        if !(state.round == round && state.step == Step::Prevote) {
            return;
        }
        state.step = Step::Precommit;

        drop(state);
        self.broadcast_and_aggregate_precommit(round, SingleDecision::Nil);
    }

    // Lines 65-67
    fn on_timeout_precommit(&self, round: u32) {
        let state = self.state.read();

        if !(state.round == round) {
            return;
        }

        drop(state);
        self.start_round(round + 1);
    }

    fn broadcast_and_aggregate_prevote(&self, round: u32, our_decision: SingleDecision) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.broadcast_and_aggregate_prevote_async(round, our_decision)
                .await;
        });
    }

    async fn broadcast_and_aggregate_prevote_async(
        &self,
        round: u32,
        our_decision: SingleDecision,
    ) {
        let proposal = self.state.read().current_proposal.clone();
        let prevote_res = self
            .deps
            .broadcast_prevote(proposal, round, &our_decision)
            .await;

        match prevote_res {
            PrevoteAggregationResult::Polka(proof) => {
                self.state.write().current_proof = Some(proof);
                self.on_polka(round);
            }
            PrevoteAggregationResult::Other(proof) => {
                self.state.write().current_proof = Some(proof);
                self.on_timeout_prevote(round);
            }
            PrevoteAggregationResult::NilPolka(proof) => {
                self.state.write().current_proof = Some(proof);
                self.on_nil_polka(round);
            }
        }
    }

    fn broadcast_and_aggregate_precommit(&self, round: u32, our_decision: SingleDecision) {
        let weak_self = self.weak_self.clone();
        tokio::spawn(async move {
            let this = upgrade_weak!(weak_self);
            this.broadcast_and_aggregate_precommit_async(round, our_decision)
                .await;
        });
    }

    async fn broadcast_and_aggregate_precommit_async(
        &self,
        round: u32,
        our_decision: SingleDecision,
    ) {
        let proposal = self.state.read().current_proposal.clone();
        let precom_res = self
            .deps
            .broadcast_precommit(proposal, round, &our_decision)
            .await;

        if let PrecommitAggregationResult::Block2Fp1(proof) = precom_res {
            self.on_2f1_block_precommits(round, proof);
        }

        self.on_timeout_precommit(round);
    }

    fn return_state_update(&self) {
        let mut state = self.state.write();

        let state_update = TendermintReturn::StateUpdate(TendermintStateProof {
            current_round: state.round,
            locked_round: state.locked_round.clone(),
            valid_round: state.valid_round.clone(),
            current_proposal: state.current_proposal.clone(),
            proof: state.current_proof.clone(),
            current_step: state.step,
        });
        drop(state);

        if let Some(return_stream) = self.state.write().return_stream.as_mut() {
            if let Err(_) = return_stream.unbounded_send(state_update) {
                warn!("Failed sending/returning tendermint state update")
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Message<P>
where
    P: Clone + Serialize + Deserialize + Send + Sync + 'static,
{
    pub proposal: P,
    pub round: u32,
    pub valid_round: u32,
}

impl<P> NetworkInterfaceMessage for Message<P>
where
    P: Clone + Serialize + Deserialize + Send + Sync + 'static,
{
    const TYPE_ID: u64 = 120;
}

#[derive(Debug, Eq, PartialEq)]
pub enum SingleDecision {
    Block,
    Nil,
}

pub enum PrecommitAggregationResult<ProofTy> {
    Block2Fp1(ProofTy),
    Other(ProofTy),
}

pub enum PrevoteAggregationResult<ProofTy> {
    Polka(ProofTy),
    NilPolka(ProofTy),
    Other(ProofTy),
}

#[async_trait]
pub trait TendermintOutsideDeps {
    type ProposalTy: Clone + Serialize + Deserialize + Send + Sync + 'static;
    type ProofTy: Clone + Send + Sync + 'static;
    type ResultTy: Send + Sync + 'static;

    fn is_our_turn(&self, round: u32) -> bool;

    fn verify_proposal(&self, proposal: Arc<Self::ProposalTy>, round: u32) -> bool;

    fn produce_proposal(&self, round: u32) -> Option<Self::ProposalTy>;

    fn assemble_block(
        &self,
        proposal: Arc<Self::ProposalTy>,
        proof: Self::ProofTy,
    ) -> Self::ResultTy;

    async fn broadcast_prevote(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrevoteAggregationResult<Self::ProofTy>;

    async fn broadcast_precommit(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrecommitAggregationResult<Self::ProofTy>;

    fn verify_proposal_state(&self, round: u32, previous_precommit_result: &Self::ProofTy) -> bool;

    fn verify_prevote_state(
        &self,
        round: u32,
        proposal: Arc<Self::ProposalTy>,
        previous_precommit_result: &Self::ProofTy,
    ) -> bool;

    fn verify_precommit_state(
        &self,
        round: u32,
        proposal: Option<Arc<Self::ProposalTy>>,
        prevote_result: &Self::ProofTy,
    ) -> Option<SingleDecision>;
}

pub const BLOCK_PROPOSAL_TIMEOUT: Duration = Duration::from_secs(1);
pub const BLOCK_PROPOSAL_TIMEOUT_DELTA: Duration = Duration::from_secs(1);
