use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use futures::{future, StreamExt};
use tokio::{sync::mpsc, time};

use bls::SecretKey;
use nimiq_block::{
    MacroBlock, MultiSignature, TendermintIdentifier, TendermintStep, TendermintVote,
};
use nimiq_handel::{identity::WeightRegistry, update::LevelUpdateMessage};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{policy, slots::Validators};
use nimiq_tendermint::{AggregationResult, TendermintError};
use nimiq_validator_network::ValidatorNetwork;

use crate::aggregation::{
    network_sink::NetworkSink, registry::ValidatorRegistry,
    tendermint::aggregations::TendermintAggregations,
};

use super::{
    background_task::BackgroundTask,
    contribution::TendermintContribution,
    utils::{AggregationEvent, CurrentAggregation},
};

/// Adaption for tendermint not using the handel stream directly. Ideally all of what this Adapter does would be done callerside just using the stream
pub struct HandelTendermintAdapter<N: ValidatorNetwork> {
    current_bests: Arc<RwLock<BTreeMap<(u32, TendermintStep), TendermintContribution>>>,
    current_aggregate: Arc<RwLock<Option<CurrentAggregation>>>,
    pending_new_round: Arc<RwLock<Option<u32>>>,
    validator_merkle_root: Vec<u8>,
    block_height: u32,
    secret_key: SecretKey,
    validator_id: u16,
    validator_registry: Arc<ValidatorRegistry>,
    network: Arc<N>,
    event_sender: mpsc::Sender<AggregationEvent<N>>,
    background_task: Option<BackgroundTask<N>>,
}

impl<N: ValidatorNetwork + 'static> HandelTendermintAdapter<N>
where
    <<N as ValidatorNetwork>::PeerType as network_interface::peer::Peer>::Id: 'static,
{
    pub fn new(
        validator_id: u16,
        active_validators: Validators,
        block_height: u32,
        network: Arc<N>,
        secret_key: SecretKey,
    ) -> Self {
        let validator_merkle_root = MacroBlock::create_pk_tree_root(&active_validators);

        // the input stream is all levelUpdateMessages concerning a TendemrintContribution and TendemrintIdentifier.
        // We get rid of the sender, but while processing these messages they need to be dispatched to the appropriate Aggregation.
        let input = Box::pin(
            network
                .receive::<LevelUpdateMessage<TendermintContribution, TendermintIdentifier>>()
                .filter_map(move |msg| {
                    future::ready(if msg.0.tag.block_number == block_height {
                        Some(msg.0)
                    } else {
                        log::debug!(
                            "Received message for different block_height: msg.0.tag.block_number: {} - actual block_height: {}",
                            msg.0.tag.block_number,
                            block_height
                        );
                        None
                    })
                }),
        );

        let validator_registry = Arc::new(ValidatorRegistry::new(active_validators));

        let (event_sender, event_receiver) = mpsc::channel::<AggregationEvent<N>>(1);

        let aggregations = TendermintAggregations::new(
            validator_id,
            validator_registry.clone(),
            input,
            event_receiver,
        );
        let current_bests = Arc::new(RwLock::new(BTreeMap::new()));
        let current_aggregate = Arc::new(RwLock::new(None));
        let pending_new_round = Arc::new(RwLock::new(None));

        let background_task = Some(BackgroundTask::new(
            aggregations,
            current_aggregate.clone(),
            current_bests.clone(),
            pending_new_round.clone(),
            validator_registry.clone(),
        ));

        Self {
            current_bests,
            current_aggregate,
            pending_new_round,
            validator_merkle_root,
            block_height,
            secret_key,
            validator_id,
            validator_registry,
            network,
            event_sender,
            background_task,
        }
    }

    /// starts an aggregation for given `round` and `step`.
    /// * `round` is the number indicating in which round Tendermint is
    /// * `step` is either `TendermintStep::PreVote` or `Tendermint::PreCommit`.
    /// * `proposal` is the hash of the proposed macro block header this node wants to vote for or `None`, if this node wishes to not vote for any block.
    pub async fn broadcast_and_aggregate(
        &mut self,
        round: u32,
        step: impl Into<TendermintStep>,
        proposal_hash: Option<Blake2bHash>,
    ) -> Result<AggregationResult<MultiSignature>, TendermintError> {
        let step = step.into();
        // make sure that there is no currently ongoing aggregation from a previous call to `broadcast_and_aggregate` which has not yet been awaited.
        // if there is none make sure to set this one with the same lock to prevent a race condition
        let (mut aggregate_receiver, _aggregate_sender) = {
            let mut current_aggregate = self
                .current_aggregate
                .write()
                .expect("current_aggregate lock could not be aquired.");

            match current_aggregate.take() {
                Some(aggr) => {
                    // re-set the current_agggregate and return error
                    *current_aggregate = Some(aggr);
                    debug!("An aggregation was started before the previous one was cleared");
                    return Err(TendermintError::AggregationError);
                }
                None => {
                    // create channel foor result propagation
                    let (sender, aggregate_receiver) =
                        mpsc::unbounded_channel::<AggregationResult<MultiSignature>>();
                    // set the current aggregate
                    *current_aggregate = Some(CurrentAggregation {
                        sender: sender.clone(),
                        round,
                        step,
                    });
                    // and return the receiver for it to be used later on.
                    (aggregate_receiver, sender)
                }
            }
        };

        // Assemble identifier from availablle information
        let id = TendermintIdentifier {
            block_number: self.block_height,
            round_number: round,
            step,
        };

        // Construct the vote so it can be hashed and signed
        let vote = TendermintVote {
            proposal_hash: proposal_hash.clone(),
            id: id.clone(),
            validator_merkle_root: self.validator_merkle_root.clone(),
        };

        // Create the signed contribution of this validator
        let own_contribution = TendermintContribution::from_vote(
            vote,
            &self.secret_key,
            self.validator_registry.get_slots(self.validator_id),
        );

        let output_sink = Box::new(NetworkSink::<
            LevelUpdateMessage<TendermintContribution, TendermintIdentifier>,
            N,
        >::new(self.network.clone()));

        // Relay the AggregationEvent to TendermintAggregations
        self.event_sender
            .send(AggregationEvent::Start(
                id.clone(),
                own_contribution,
                self.validator_merkle_root.clone(),
                output_sink,
            ))
            .await
            .map_err(|err| {
                debug!("event_sender.send failed: {:?}", err);
                TendermintError::AggregationError
            })?;

        // If a new round event was emitted before it needs to be checked if it is still relevant by
        // checking if it concerned a round higher than the one which is currently starting.
        if let Some(pending_round) = self
            .pending_new_round
            .write()
            .expect("pending_new_round lock could not be aquired.")
            .take()
        {
            if pending_round > round {
                // If it is higher then the NewRound Event is emitted finishing this broadcast_and_aggregate call.
                // The aggregation itself however willl remain valid, as it might get referenced in a future round.
                self.current_aggregate
                    .write()
                    .expect("current_aggregate lock could not be aquired.")
                    .take();

                return Ok(AggregationResult::NewRound(pending_round));
            }
        }

        // if there is no new Round to return, proceed with waiting for the Event

        // wait for the first result
        let mut result = match aggregate_receiver.recv().await {
            Some(event) => event,
            None => {
                debug!("The aggregate_receiver could not receive an item");
                return Err(TendermintError::AggregationError);
            }
        };

        // create timeout according to rules. For every consecutive round the timeout must increase by a constant factor.
        // TODO constants
        let deadline = time::Instant::now()
            .checked_add(time::Duration::from_millis(100u64 + round as u64 * 10u64))
            .expect("Cannot create timeout Instant");

        loop {
            // check if the current result is final (no improvment possible even though ot all signatories have siged) if so return it immediately
            // There are some conditions for that:
            // * one single proposal has 2f+1 vote weight
            // * the nil proposal has f+1 (no other individual proposal can ever reach 2f+1, since in that case as only 2f remain)
            // * all proposals combined (including nil) except the proposal this node signed have 2f+1 vote weight
            //      (as this can never result in a block vote, only in nil vote)
            // 1* f+1 for a future round (`TendermintAggregationEvent::NewRound(n)`)
            match &result {
                // If the event is a NewRound it is propagated as is.
                AggregationResult::NewRound(round) => {
                    if step == TendermintStep::PreCommit {
                        // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                        self.event_sender
                            .send(AggregationEvent::Cancel(*round, step))
                            .await
                            .map_err(|err| {
                                debug!("event_sender.send failed: {:?}", err);
                                TendermintError::AggregationError
                            })?;
                    }

                    debug!("Aggregations returned a NewRound({})", round);
                    return Ok(result);
                }

                // If the event is an Aggregation there are some comparisons to be made before it can be returned (or not)
                AggregationResult::Aggregation(map) => {
                    // keep trck of the combined weight of all proposals which this node has not signed.
                    let mut combined_weight = 0usize;
                    let mut total_weight = 0usize;

                    // iterate all proposals present in this contribution
                    for (proposal, (_, weight)) in map.iter() {
                        if *weight > policy::TWO_THIRD_SLOTS as usize {
                            if step == TendermintStep::PreCommit {
                                // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                                self.event_sender
                                    .send(AggregationEvent::Cancel(round, step))
                                    .await
                                    .map_err(|err| {
                                        debug!("event_sender.send failed: {:?}", err);
                                        TendermintError::AggregationError
                                    })?;
                            }
                            trace!("Tendermint: {}-{:?}: A proposal has > 2f+1", &round, &step);
                            return Ok(result);
                        }

                        // if this node has signed a different proposal this vote counts towards the combined weight
                        if proposal != &proposal_hash {
                            combined_weight += weight;
                        }

                        total_weight += weight;
                    }

                    // combined weight of all proposals excluding the one this node signed reached 2f+1
                    if combined_weight > policy::TWO_THIRD_SLOTS as usize {
                        if step == TendermintStep::PreCommit {
                            // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                            self.event_sender
                                .send(AggregationEvent::Cancel(round, step))
                                .await
                                .map_err(|err| {
                                    debug!("event_sender.send failed: {:?}", err);
                                    TendermintError::AggregationError
                                })?;
                        }
                        trace!(
                            "Tendermint: {}-{:?}: All other proposals have > 2f + 1 votes",
                            &round,
                            &step
                        );
                        return Ok(result);
                    }

                    // none of the above but every signatory is present and thus no improvement can be made
                    if total_weight == policy::SLOTS as usize {
                        if step == TendermintStep::PreCommit {
                            // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                            self.event_sender
                                .send(AggregationEvent::Cancel(round, step))
                                .await
                                .map_err(|err| {
                                    debug!("event_sender.send failed: {:?}", err);
                                    TendermintError::AggregationError
                                })?;
                        }
                        trace!("Tendermint: {}-{:?}: Everybody signed", &round, &step);
                        return Ok(result);
                    }
                }
            }

            debug!("Tendermint: {}-{:?}: timeout triggered", &round, &step);
            // if the result is not immediately actionable wait for a new (and better) result to check again.
            // Only wait for a set period of time, if it elapses the current (best) result is returned (likely resulting in a subsequent Nil vote/commit).
            match time::timeout_at(deadline, aggregate_receiver.recv()).await {
                Ok(Some(event)) => result = event,
                Ok(None) => return Err(TendermintError::AggregationError),
                Err(_) => {
                    if step == TendermintStep::PreCommit {
                        // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                        self.event_sender
                            .send(AggregationEvent::Cancel(round, step))
                            .await
                            .map_err(|err| {
                                debug!("event_sender.send failed: {:?}", err);
                                TendermintError::AggregationError
                            })?;
                    }
                    return Ok(result);
                }
            }
        }
    }

    pub fn get_aggregate(
        &self,
        round: u32,
        step: impl Into<TendermintStep>,
    ) -> Result<AggregationResult<MultiSignature>, TendermintError> {
        if let Some(current_best) = self
            .current_bests
            .read()
            .expect("current_bests lock could not be aquired.")
            .get(&(round, step.into()))
        {
            Ok(AggregationResult::Aggregation(
                current_best
                    .contributions
                    .iter()
                    .map(|(hash, contribution)| {
                        (
                            hash.clone(),
                            (
                                contribution.clone(),
                                self.validator_registry
                                    .signature_weight(contribution)
                                    .expect("Cannot compute weight of signatories"),
                            ),
                        )
                    })
                    .collect(),
            ))
        } else {
            Err(TendermintError::AggregationDoesNotExist)
        }
    }

    pub fn create_background_task(&mut self) -> BackgroundTask<N> {
        self.background_task
            .take()
            .expect("The background stream cannot be creaed twice.")
    }
}
