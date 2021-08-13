use std::{
    collections::{btree_map::Entry, BTreeMap},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll,
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
pub(super) struct TendermintAggregations<N: ValidatorNetwork> {
    event_receiver: mpsc::Receiver<AggregationEvent<N>>,
    combined_aggregation_streams:
        Pin<Box<SelectAll<BoxStream<'static, ((u32, TendermintStep), TendermintContribution)>>>>,
    aggregation_descriptors: BTreeMap<(u32, TendermintStep), AggregationDescriptor>,
    input: BoxStream<'static, LevelUpdateMessage<TendermintContribution, TendermintIdentifier>>,
    future_aggregations: BTreeMap<u32, BitSet>,
    validator_id: u16,
    validator_registry: Arc<ValidatorRegistry>,
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
        }
    }

    fn broadcast_and_aggregate<E: std::fmt::Debug + Send + 'static>(
        &mut self,
        id: TendermintIdentifier,
        own_contribution: TendermintContribution,
        validator_merkle_root: Vec<u8>,
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
                validator_merkle_root,
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

            trace!(
                "Aggregation_descriptors: {:?}",
                &self.aggregation_descriptors
            );

            // copy round_number for use in drain_filter couple of lines down so that id can be moved into closure.
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
                .drain_filter(|round, _bitset| round <= &round_number);

            // Push the aggregation to the select_all streams.
            self.combined_aggregation_streams.push(aggregation);
        }
    }

    pub fn cancel_aggregation(&self, round: u32, step: TendermintStep) {
        if let Some(descriptor) = self.aggregation_descriptors.get(&(round, step)) {
            trace!("canceling aggregation for {}-{:?}", &round, &step);
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
        match self.event_receiver.poll_recv(cx) {
            Poll::Pending => {}
            Poll::Ready(None) => {
                error!("Event receiver created None value"); // <---
                                                             // return Poll::Ready(None);
            }
            Poll::Ready(Some(AggregationEvent::Start(
                id,
                own_contribution,
                validator_merkle_root,
                output_sink,
            ))) => self.broadcast_and_aggregate(
                id,
                own_contribution,
                validator_merkle_root,
                output_sink,
            ),
            Poll::Ready(Some(AggregationEvent::Cancel(round, step))) => {
                self.cancel_aggregation(round, step)
            }
        }

        // empty out the input stream dispatching messages to the appropriate aggregations
        while let Poll::Ready(message) = self.input.poll_next_unpin(cx) {
            if let Some(message) = message {
                if let Some(descriptor) = self
                    .aggregation_descriptors
                    .get(&(message.tag.round_number, message.tag.step))
                {
                    trace!("New message for ongoing aggregation: {:?}", &message);
                    if descriptor.input.send(message.update).is_err() {
                        debug!("Failed to relay LevelUpdate to aggregation");
                    }
                } else if let Some(((highest_round, _), _)) =
                    self.aggregation_descriptors.last_key_value()
                {
                    // messages of future rounds need to be tracked in terrms of contributors only (without verifying them).
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
                            if weight > policy::SLOTS as usize - policy::TWO_THIRD_SLOTS as usize {
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
            }
            /*
            The code inside the `else {}` was commented before, so the `else {}` case was not doing anything anymore,
            which means it made sense to refactor this from a `match` to an `if let`. I'm leaving the code here in case
            the person who originally commented the code inside the `else {}` may need it in the future.

            ```
            else {
             Poll::Ready(None) means the stream has terminated, which is likelt the network having droped.
             Try and reconnect but also log an error
             error!("Networks receive from all returned Poll::Ready(None)");
             return Poll::Ready(None);
            }
            ```
            */
        }
        // after that return whatever combined_aggregation_streams returns
        match self.combined_aggregation_streams.poll_next_unpin(cx) {
            Poll::Pending | Poll::Ready(None) => Poll::Pending,
            Poll::Ready(Some(((round, step), aggregation))) => Poll::Ready(Some(
                TendermintAggregationEvent::Aggregation(round, step, aggregation),
            )),
        }
    }
}
