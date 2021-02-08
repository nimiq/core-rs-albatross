use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time;

use futures::future;
use futures::sink::Sink;
use futures::stream::{BoxStream, SelectAll, Stream, StreamExt};
use futures::task::{Context, Poll};

use futures_locks::RwLock;

use nimiq_block_albatross::{create_pk_tree_root, MultiSignature, TendermintIdentifier, TendermintStep, TendermintVote};
use nimiq_bls::SecretKey;
use nimiq_collections::bitset::BitSet;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;
use nimiq_primitives::slot::ValidatorSlots;

use nimiq_handel::aggregation::Aggregation;
use nimiq_handel::config::Config;
use nimiq_handel::contribution::AggregatableContribution;
use nimiq_handel::identity::WeightRegistry;
use nimiq_handel::update::{LevelUpdate, LevelUpdateMessage};

use nimiq_tendermint::{AggregationResult, TendermintError};
use nimiq_validator_network::ValidatorNetwork;

use super::super::network_sink::NetworkSink;
use super::super::registry::ValidatorRegistry;

use super::contribution::TendermintContribution;
use super::protocol::TendermintAggregationProtocol;
use super::utils::{AggregationDescriptor, CurrentAggregation, TendermintAggregationEvent};

/// Used to pass events from HandelTendermintAdapter to and from TendermintAggregations
enum AggregationEvent<N: ValidatorNetwork> {
    Start(
        TendermintIdentifier,
        TendermintContribution,
        Vec<u8>,
        Box<NetworkSink<LevelUpdateMessage<TendermintContribution, TendermintIdentifier>, N>>,
    ),
    Cancel(
        u32,
        TendermintStep,
    ),
}

impl<N: ValidatorNetwork> std::fmt::Debug for AggregationEvent<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AggregationEvent::Start(i, _, _, _) => f.debug_struct("Start").field("id",i).finish(),
            AggregationEvent::Cancel(r, s) => f.debug_struct("Start").field("id", &(r, s)).finish(),
        }
    }
}

/// Maintains various aggregations for different rounds and steps of Tendermint.
///
/// Note that `TendermintAggregations::broadcast_and_aggregate` needs to have been called at least once before the stream can meaningfully be awaited.
pub struct TendermintAggregations<N: ValidatorNetwork> {
    event_receiver: mpsc::Receiver<AggregationEvent<N>>,
    combined_aggregation_streams: Pin<Box<SelectAll<BoxStream<'static, ((u32, TendermintStep), TendermintContribution)>>>>,
    aggregation_descriptors: BTreeMap<(u32, TendermintStep), AggregationDescriptor>,
    input: BoxStream<'static, LevelUpdateMessage<TendermintContribution, TendermintIdentifier>>,
    future_aggregations: BTreeMap<u32, BitSet>,
    validator_id: u16,
    validator_registry: Arc<ValidatorRegistry>,
}

impl<N: ValidatorNetwork> TendermintAggregations<N> {
    fn new(
        validator_id: u16,
        validator_registry: Arc<ValidatorRegistry>,
        input: BoxStream<'static, LevelUpdateMessage<TendermintContribution, TendermintIdentifier>>,
        event_receiver: mpsc::Receiver<AggregationEvent<N>>,
    ) -> Self {
        // Create the instance and return it
        TendermintAggregations {
            combined_aggregation_streams: Box::pin(SelectAll::new()),
            aggregation_descriptors: BTreeMap::new(),
            future_aggregations: BTreeMap::new(),
            input,
            validator_id,
            validator_registry,
            event_receiver,
        }
    }

    fn broadcast_and_aggregate<E: std::fmt::Debug + Send + 'static>(
        &mut self,
        id: TendermintIdentifier,
        own_contribution: TendermintContribution,
        validator_merkle_root: Vec<u8>,
        output_sink: Box<(dyn Sink<(LevelUpdateMessage<TendermintContribution, TendermintIdentifier>, usize), Error = E> + Unpin + Send)>,
    ) {
        // TODO: TendermintAggregationEvent
        if !self.aggregation_descriptors.contains_key(&(id.round_number, id.step)) {
            debug!("starting aggregation for {:?}", &id);
            // crate the correct protocol instance
            let protocol = TendermintAggregationProtocol::new(
                self.validator_registry.clone(),
                self.validator_id as usize,
                1, // To be removed
                id.clone(),
                validator_merkle_root,
            );

            let (sender, receiver) = mpsc::unbounded_channel::<LevelUpdate<TendermintContribution>>();

            // create the aggregation
            let aggregation = Aggregation::new(protocol, id.clone(), Config::default(), own_contribution, receiver.boxed(), output_sink);

            // create the stream closer and wrap in Arc so it can be shared borrow
            let stream_closer = Arc::new(AtomicBool::new(true));

            // Create and store AggregationDescriptor
            self.aggregation_descriptors.insert(
                (id.round_number, id.step),
                AggregationDescriptor {
                    input: sender,
                    is_running: stream_closer.clone(),
                },
            );

            // copy round_number for use in drain_filter couple of lines down so that id can be moved into closure.
            let round_number = id.round_number;

            // wrap the aggregation stream
            let aggregation = Box::pin(aggregation.map(move |x| ((id.round_number, id.step), x)).take_while(move |_x| {
                future::ready(stream_closer.clone().load(Ordering::Relaxed))
                // Todo: Check Ordering
            }));

            // Since this instance of Aggregation now becomes the current aggregation all bitsets containing contributors
            // for future aggregations which are older or same age than this one can be discarded.
            self.future_aggregations.drain_filter(|round, _bitset| round <= &round_number);

            // Push the aggregation to the select_all streams.
            self.combined_aggregation_streams.push(aggregation);
        }
    }

    pub fn cancel_aggregation(&self, round: u32, step: TendermintStep) {
        if let Some(descriptor) = self.aggregation_descriptors.get(&(round, step)) {
            debug!("canceling aggregation for {}-{:?}", &round, &step);
            descriptor.is_running.store(false, Ordering::Relaxed);
        }
    }
}

impl<N: ValidatorNetwork + 'static> Stream for TendermintAggregations<N> {
    type Item = TendermintAggregationEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.event_receiver.poll_recv(cx) {
            Poll::Pending => {},
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Ready(Some(AggregationEvent::Start(
                id,
                own_contribution,
                validator_merkle_root,
                output_sink,
            ))) => self.broadcast_and_aggregate(id, own_contribution, validator_merkle_root, output_sink),
            Poll::Ready(Some(AggregationEvent::Cancel(
                round,
                step,
            ))) => self.cancel_aggregation(round, step),
        }

        // empty out the input stream dispatching messages tothe appropriate aggregations
        while let Poll::Ready(message) = self.input.poll_next_unpin(cx) {
            match message {
                Some(message) => {
                    if let Some(descriptor) = self.aggregation_descriptors.get(&(message.tag.round_number, message.tag.step)) {
                        trace!("New message for ongoing aggregation: {:?}", &message);
                        if descriptor.input.send(message.update).is_err() {
                             error!("Failed to relay LevelUpdate to aggregation");
                        }
                    } else if let Some(((highest_round, _), _)) = self.aggregation_descriptors.last_key_value() {
                        // messages of future rounds need to be tracked in terrms of contributors only (without verifying them).
                        // Also note that PreVote and PreCommit are tracked in the same bitset as the protocol requires.
                        if highest_round < &message.tag.round_number {
                            debug!("New contribution for future round: {:?}", &message);
                            let future_contributors = self
                                .future_aggregations
                                .entry(message.tag.round_number)
                                .and_modify(|bitset| *bitset |= message.update.aggregate.contributors())
                                .or_insert(message.update.aggregate.contributors())
                                .clone();
                            // now check if that suffices for a f+1 contributor weight
                            if let Some(weight) = self.validator_registry.signers_weight(&future_contributors) {
                                if weight > policy::SLOTS as usize - policy::TWO_THIRD_SLOTS as usize {
                                    return Poll::Ready(Some(TendermintAggregationEvent::NewRound(message.tag.round_number)));
                                }
                            }
                        }
                    } else {
                        // No last_key_value means there is no aggregation whatsoever.
                        // Discard the update as there is no receiver for it and log to debug.
                        debug!("Found no agggregations, but received a LevelUpdateMessage: {:?}", &self.aggregation_descriptors);
                    }
                }
                // Poll::Ready(None) means the stream has terminated, which is likelt the network having droped.
                // Try and reconnect but also log an error
                None => {
                    error!("Networks receive from all returned Poll::Ready(None)");
                }
            }
        }
        // after that return whatever combined_aggregation_streams returns
        match self.combined_aggregation_streams.poll_next_unpin(cx) {
            Poll::Pending | Poll::Ready(None) => Poll::Pending,
            Poll::Ready(Some(((round, step), aggregation))) => Poll::Ready(Some(TendermintAggregationEvent::Aggregation(round, step, aggregation))),
        }
    }
}

/// Adaption for tendermint not using the handel stream directly. Ideally all of what this Adapter does would be done callerside just using the stream
pub struct HandelTendermintAdapter<N: ValidatorNetwork> {
    current_bests: RwLock<BTreeMap<(u32, TendermintStep), TendermintContribution>>,
    current_aggregate: RwLock<Option<CurrentAggregation>>,
    pending_new_round: RwLock<Option<u32>>,
    validator_merkle_root: Vec<u8>,
    block_height: u32,
    secret_key: SecretKey,
    validator_id: u16,
    validator_registry: Arc<ValidatorRegistry>,
    network: Arc<N>,
    event_sender: mpsc::Sender<AggregationEvent<N>>,
}

impl<N: ValidatorNetwork + 'static> HandelTendermintAdapter<N>
where <<N as ValidatorNetwork>::PeerType as network_interface::peer::Peer>::Id: 'static {
    pub fn new(validator_id: u16, active_validators: ValidatorSlots, block_height: u32, network: Arc<N>, secret_key: SecretKey) -> Self {
        let validator_merkle_root = create_pk_tree_root(&active_validators);

        // the input stream is all levelUpdateMessages concerning a TendemrintContribution and TendemrintIdentifier.
        // We get rid of the sender, but while processing these messages they need to be dispatched to the appropriate Aggregation.
        let input = Box::pin(
            network
                .receive::<LevelUpdateMessage<TendermintContribution, TendermintIdentifier>>()
                .map(move |msg| msg.0),
        );

        let validator_registry = Arc::new(ValidatorRegistry::new(active_validators));

        let (event_sender, event_receiver) = mpsc::channel::<AggregationEvent<N>>(1);

        let mut aggregations = TendermintAggregations::new(validator_id, validator_registry.clone(), input, event_receiver);
        let current_bests = RwLock::new(BTreeMap::new());
        let current_aggregate = RwLock::new(None);
        let pending_new_round = RwLock::new(None);

        let this = Self {
            current_bests: current_bests.clone(),
            current_aggregate: current_aggregate.clone(),
            pending_new_round: pending_new_round.clone(),
            validator_merkle_root,
            block_height,
            secret_key,
            validator_id,
            validator_registry: validator_registry.clone(),
            network,
            event_sender,
        };

        // Here all the streams are polled to get each and every new aggregate and new round event.
        // Special care needs to be taken to release locks as early as possible
        tokio::spawn(async move {
            loop {
                if let Some(event) = aggregations.next().await {
                    match event {
                        TendermintAggregationEvent::NewRound(next_round) => {
                            // This needs to be forwarded to a waiting current_aggregate sink if there is one.
                            // if there is none, the highest round received in a TendermintAggregationEvent::NewRound(n) must be kept
                            // such that it can be send to the next current_aggregate sink if it by then is still relevant.
                            match current_aggregate.write().await.take() {
                                // if there is a current ongoing aggregation it is retrieved
                                Some(CurrentAggregation { sender, round, step: _step }) => {
                                    // check that the next_round is actually higher than the round which is awaiting a result.
                                    if round < next_round {
                                        // Send the me NewRound event through the channel to the currently awaited aggregation for it to resolve.
                                        if let Err(_err) = sender.send(AggregationResult::NewRound(next_round)) {
                                            error!("Sending of AggregationEvent::NewRound({}) failed", round);
                                        }
                                    } else {
                                        // Otheriwse this event is meaningless and is droped, which in reality should never occur.
                                        // trace it for debugging purposes.
                                        trace!("received NewRound({}), but curret_aggregate is in round {}", next_round, round);
                                    }
                                }
                                // if there is no caller awaiting a result
                                None => {
                                    // see if the pending_new_round field is actually set already
                                    let mut lock = pending_new_round.write().await;
                                    match lock.take() {
                                        Some(current_round) => {
                                            // if it is set the bigger of the existing value and the newly received one is maintained.
                                            if current_round < next_round {
                                                *lock = Some(next_round);
                                            } else {
                                                *lock = Some(current_round);
                                            }
                                        }
                                        None => {
                                            // since there is no new_round pending, the received value is set as such.
                                            *lock = Some(next_round);
                                        }
                                    };
                                }
                            };
                        }
                        TendermintAggregationEvent::Aggregation(r, s, c) => {
                            debug!("New Aggregate for {}-{:?}: {:?}", &r, &s, &c);
                            // There is a new aggregate, check if it is a better aggregate for (round, step) than we curretly have
                            // if so, store it.
                            let contribution = current_bests
                                .write()
                                .await
                                .entry((r, s))
                                .and_modify(|contribution| {
                                    // this test is not strictly necessary as the handel streams will only return a value if it is better  than the previous one.
                                    if validator_registry.signature_weight(contribution).expect("Failed to unwrap signature weight")
                                        < validator_registry.signature_weight(&c).expect("Failed to unwrap signature weight")
                                    {
                                        *contribution = c.clone();
                                    }
                                })
                                .or_insert(c)
                                .clone();

                            // better or not, we need to check if it is actionable for tendermint, and if it is and there is a
                            // current_aggregate sink present, we send it as well.
                            let mut lock = current_aggregate.write().await;
                            match lock.take() {
                                // if there is a current ongoing aggregation it is retrieved
                                Some(CurrentAggregation { sender, round, step }) => {
                                    // only reply if it is the correct aggregation
                                    if round == r && step == s {
                                        // see if this is actionable
                                        // Actionable here means everything with voter_weight > 2f+1 is potentially actionable.
                                        // On the receiving end of this channel is a timeout, which will wait (if necessary) for better results
                                        // but will, if nothing better is made available also return this aggregate.
                                        if validator_registry.signature_weight(&contribution).expect("Failed to unwrap signature weight")
                                            > policy::TWO_THIRD_SLOTS as usize
                                        {
                                            // Transform to the appropiate result type adding the weight to each proposals Contribution
                                            let result = contribution
                                                .contributions
                                                .iter()
                                                .map(|(hash, contribution)| {
                                                    (
                                                        hash.clone(),
                                                        (
                                                            contribution.clone(),
                                                            validator_registry.signature_weight(contribution).expect("Cannot compute weight of signatories"),
                                                        ),
                                                    )
                                                })
                                                .collect();

                                            // send the result
                                            if let Err(err) = sender.send(AggregationResult::Aggregation(result)) {
                                                error!("failed sending message to broadcast_and_aggregate caller: {:?}", err);
                                                // recoverable?
                                            }
                                        } else {
                                            // re-set the current_aggregate
                                            *lock = Some(CurrentAggregation { sender, round, step });
                                        }
                                    } else {
                                        // re-set the current_aggregate
                                        *lock = Some(CurrentAggregation { sender, round, step });
                                    }
                                }
                                None => {
                                    debug!("there was no receiver");
                                    // do nothing, as there is no caller awaiting a response.
                                }
                            };
                        }
                    };
                } else {
                    // Once there is no items left on the combined handel stream then there is nothing being aggregated
                    // so this task is finished and will terminate.
                    debug!("Finished Aggregating");
                    break;
                    // TODO how to terminate this
                }
            }
        });

        this
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
            let mut current_aggregate = self.current_aggregate.write().await;
            match current_aggregate.take() {
                Some(aggr) => {
                    // re-set the current_agggregate and return error
                    *current_aggregate = Some(aggr);
                    return Err(TendermintError::AggregationError);
                }
                None => {
                    // create channel foor result propagation
                    let (sender, aggregate_receiver) = mpsc::unbounded_channel::<AggregationResult<MultiSignature>>();
                    // set the current aggregate
                    *current_aggregate = Some(CurrentAggregation { sender: sender.clone(), round, step });
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
        let own_contribution = TendermintContribution::from_vote(vote, &self.secret_key, self.validator_registry.get_slots(self.validator_id));

        let output_sink = Box::new(NetworkSink::<LevelUpdateMessage<TendermintContribution, TendermintIdentifier>, N>::new(
            self.network.clone(),
        ));

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
        if let Some(pending_round) = self.pending_new_round.write().await.take() {
            if pending_round > round {
                // If it is higher then the NewRound Event is emitted finishing this broadcast_and_aggregate call.
                // The aggregation itself however willl remain valid, as it might get referenced in a future round.
                self.current_aggregate.write().await.take();
                return Ok(AggregationResult::NewRound(pending_round));
            }
        }

        // if there is no new Round to return, proceed with waiting for the Event

        // wait for the first result
        let mut result = match aggregate_receiver.recv().await {
            Some(event) => event,
            None => {
                debug!("The aggregate_receiver could not receive an item");
                return Err(TendermintError::AggregationError)
            },
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

                    // iterate all proposals present in this contribution
                    for (proposal, (_, weight)) in map.iter() {
                        if proposal.is_none() {
                            // Nil vote has f+1
                            // if *weight > policy::SLOTS as usize - policy::TWO_THIRD_SLOTS as usize + 1usize {
                            //     if step == TendermintStep::PreCommit {
                            //         // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                            //         self.event_sender
                            //             .send(AggregationEvent::Cancel(round, step))
                            //             .await
                            //             .map_err(|err| {
                            //                 debug!("event_sender.send failed: {:?}", err);
                            //                 TendermintError::AggregationError
                            //             })?;
                            //     }
                            //     error!("Tendermint: {}-{:?}: Nil vote > f+1", &round, &step);
                            //     return Ok(result);
                            // }

                            // if this node has signed a proposal not nil this nil vote counts towards the combined weight
                            if proposal_hash.is_some() {
                                combined_weight += weight;
                            }
                        } else {
                            // any individual proposal got 2f+1 vote weight
                            if *weight > policy::TWO_THIRD_SLOTS as usize {
                                if step == TendermintStep::PreCommit {
                                    // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                                    self.event_sender
                                        .send(AggregationEvent::Cancel(round, step))
                                        .await
                                        .map_err(|_| TendermintError::AggregationError)?;
                                }
                                return Ok(result);
                            }
                            // if proposal is not the same as the one this node signed, then it contributes to the combined weight
                            if proposal != &proposal_hash {
                                combined_weight += weight;
                            }
                        }
                    }

                    // combined weight of all proposals exclluding the one this node signed reached 2f+1
                    if combined_weight > policy::SLOTS as usize {
                        if step == TendermintStep::PreCommit {
                            // PreCommit Aggreations are never requested again, so the aggregation can be canceled.
                            self.event_sender
                                .send(AggregationEvent::Cancel(round, step))
                                .await
                                .map_err(|_| TendermintError::AggregationError)?;
                        }
                        return Ok(result);
                    }
                }
            }

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
                            .map_err(|_| TendermintError::AggregationError)?;
                    }
                    return Ok(result);
                }
            }
        }
    }

    pub async fn get_aggregate(&self, round: u32, step: impl Into<TendermintStep>) -> Result<AggregationResult<MultiSignature>, TendermintError> {
        if let Some(current_best) = self.current_bests.read().await.get(&(round, step.into())) {
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
}
