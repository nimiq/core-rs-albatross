use std::{
    collections::{btree_map::Entry, BTreeMap},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Poll, Waker},
};

use futures::{
    future,
    stream::{BoxStream, SelectAll},
    Sink, Stream, StreamExt,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use nimiq_block::{TendermintIdentifier, TendermintStep};
use nimiq_collections::BitSet;
use nimiq_handel::update::LevelUpdateMessage;
use nimiq_handel::{
    aggregation::Aggregation, config::Config, contribution::AggregatableContribution,
    identity::WeightRegistry, update::LevelUpdate,
};
use nimiq_macros::store_waker;
use nimiq_primitives::policy;
use nimiq_validator_network::ValidatorNetwork;

use crate::aggregation::{
    registry::ValidatorRegistry, tendermint::protocol::TendermintAggregationProtocol,
};

use super::{
    contribution::TendermintContribution,
    utils::{AggregationDescriptor, AggregationEvent, TendermintAggregationEvent},
};

/// Maintains various aggregations for different rounds and steps of Tendermint.
///
/// Note that `TendermintAggregations::broadcast_and_aggregate` needs to have been called at least once before the stream can meaningfully be awaited.
type RoundAndStep = (u32, TendermintStep);
type RoundStepAndContribution = (RoundAndStep, TendermintContribution);

pub(super) struct TendermintAggregations<N: ValidatorNetwork> {
    event_receiver: mpsc::Receiver<AggregationEvent<N>>,
    combined_aggregation_streams: Pin<Box<SelectAll<BoxStream<'static, RoundStepAndContribution>>>>,
    aggregation_descriptors: BTreeMap<RoundAndStep, AggregationDescriptor>,
    input: BoxStream<'static, LevelUpdateMessage<TendermintContribution, TendermintIdentifier>>,
    future_aggregations: BTreeMap<u32, BitSet>,
    validator_id: u16,
    validator_registry: Arc<ValidatorRegistry>,
    /// The waker used to wake in case a new Stream is pushed into `self.combined_aggregation_streams`
    /// when there previously was none
    waker: Option<Waker>,
}

impl<N: ValidatorNetwork> TendermintAggregations<N> {
    pub fn new(
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
            // The waker can be none even though the SelectAll `self.combined_aggregation_streams` is empty
            // because the first poll to it will register the waker if it is still empty at that point.
            waker: None,
        }
    }

    fn broadcast_and_aggregate<E: std::fmt::Debug + Send + 'static>(
        &mut self,
        id: TendermintIdentifier,
        own_contribution: TendermintContribution,
        output_sink: Box<
            (dyn Sink<
                (
                    LevelUpdateMessage<TendermintContribution, TendermintIdentifier>,
                    usize,
                ),
                Error = E,
            > + Unpin
                 + Send),
        >,
    ) {
        // TODO: TendermintAggregationEvent
        if let Entry::Vacant(entry) = self
            .aggregation_descriptors
            .entry((id.round_number, id.step))
        {
            trace!("starting aggregation for {:?}", &id);
            // crate the correct protocol instance
            let protocol = TendermintAggregationProtocol::new(
                self.validator_registry.clone(),
                self.validator_id as usize,
                1, // To be removed
                id.clone(),
            );

            let (sender, receiver) =
                mpsc::unbounded_channel::<LevelUpdate<TendermintContribution>>();

            // create the aggregation
            let aggregation = Aggregation::new(
                protocol,
                id.clone(),
                Config::default(),
                own_contribution,
                Box::pin(UnboundedReceiverStream::new(receiver)),
                output_sink,
            );

            // create the stream closer and wrap in Arc so it can be shared borrow
            let stream_closer = Arc::new(AtomicBool::new(true));

            // Create and store AggregationDescriptor
            entry.insert(AggregationDescriptor {
                input: sender,
                is_running: stream_closer.clone(),
            });

            let tmp_desc: Vec<((&u32, &TendermintStep), bool)> = self
                .aggregation_descriptors
                .iter()
                .map(|((r, s), v)| ((r, s), v.is_running.load(Ordering::Relaxed)))
                .collect();

            trace!("Aggregation_descriptors: {:?}", &tmp_desc,);

            // copy round_number for use in `retain` couple of lines down so that id can be moved into closure.
            let round_number = id.round_number;

            // wrap the aggregation stream
            let aggregation = Box::pin(
                aggregation
                    .map(move |x| ((id.round_number, id.step), x))
                    .take_while(move |_x| {
                        future::ready(stream_closer.clone().load(Ordering::Relaxed))
                        // Todo: Check Ordering
                    }),
            );

            // Since this instance of Aggregation now becomes the current aggregation all bitsets containing contributors
            // for future aggregations which are older or same age than this one can be discarded.
            self.future_aggregations
                .retain(|round, _bitset| round > &round_number);

            // Push the aggregation to the select_all streams.
            self.combined_aggregation_streams.push(aggregation);
            // If a waker is registered, wake it up.
            if let Some(waker) = self.waker.take() {
                waker.wake();
            }
        }
    }

    pub fn cancel_aggregation(&self, round: u32, step: TendermintStep) {
        if let Some(descriptor) = self.aggregation_descriptors.get(&(round, step)) {
            trace!("cancelling aggregation for {}-{:?}", &round, &step);
            descriptor.is_running.store(false, Ordering::Relaxed);
        }
    }
}

impl<N: ValidatorNetwork + 'static> Stream for TendermintAggregations<N> {
    type Item = TendermintAggregationEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(Some(event)) = self.event_receiver.poll_recv(cx) {
            match event {
                AggregationEvent::Start(id, own_contribution, output_sink) => {
                    self.broadcast_and_aggregate(id, own_contribution, output_sink)
                }
                AggregationEvent::Cancel(round, step) => self.cancel_aggregation(round, step),
            }
        }

        // empty out the input stream dispatching messages to the appropriate aggregations
        while let Poll::Ready(message) = self.input.poll_next_unpin(cx) {
            if let Some(message) = message {
                if let Some(descriptor) = self
                    .aggregation_descriptors
                    .get(&(message.tag.round_number, message.tag.step))
                {
                    if descriptor.input.send(message.update.clone()).is_err() {
                        // channel was closed, thus the aggregation was terminated. Remove the descriptor.
                        self.aggregation_descriptors
                            .remove(&(message.tag.round_number, message.tag.step));
                    }
                } else if let Some((highest_round, _)) =
                    self.aggregation_descriptors.keys().next_back()
                {
                    // messages of future rounds need to be tracked in terms of contributors only (without verifying them).
                    // Also note that PreVote and PreCommit are tracked in the same bitset as the protocol requires.
                    if highest_round < &message.tag.round_number {
                        trace!("New contribution for future round: {:?}", &message);
                        let future_contributors = self
                            .future_aggregations
                            .entry(message.tag.round_number)
                            .and_modify(|bitset| *bitset |= message.update.aggregate.contributors())
                            .or_insert_with(|| message.update.aggregate.contributors())
                            .clone();
                        // now check if that suffices for a f+1 contributor weight
                        if let Some(weight) =
                            self.validator_registry.signers_weight(&future_contributors)
                        {
                            if weight >= policy::F_PLUS_ONE as usize {
                                return Poll::Ready(Some(TendermintAggregationEvent::NewRound(
                                    message.tag.round_number,
                                )));
                            }
                        }
                    }
                } else {
                    // No last_key_value means there is no aggregation whatsoever.
                    // Discard the update as there is no receiver for it and log to debug.
                    debug!(
                        "Found no agggregations, but received a LevelUpdateMessage: {:?}",
                        &self.aggregation_descriptors
                    );
                }
            } else {
                log::warn!("Input for Aggregations returned None");
            }
        }

        // after that return whatever combined_aggregation_streams returns
        match self.combined_aggregation_streams.poll_next_unpin(cx) {
            Poll::Pending => {
                store_waker!(self, waker, cx);
                Poll::Pending
            }
            Poll::Ready(None) => {
                // In case the Selectisempty it will return Poll::Ready(None).
                // Since the task will not be woken up by just adding a new stream to SelectAll
                // a waker must be stored to trigger the wake.
                store_waker!(self, waker, cx);
                Poll::Pending
            }
            Poll::Ready(Some(((round, step), aggregation))) => Poll::Ready(Some(
                TendermintAggregationEvent::Aggregation(round, step, aggregation),
            )),
        }
    }
}
