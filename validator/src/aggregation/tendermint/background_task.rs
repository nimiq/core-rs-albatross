use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::task::Poll;

use futures::{ready, StreamExt};
use parking_lot::RwLock;

use nimiq_block::TendermintStep;
use nimiq_handel::contribution::AggregatableContribution;
use nimiq_handel::identity::WeightRegistry;
use nimiq_primitives::policy;
use nimiq_tendermint::AggregationResult;
use nimiq_validator_network::ValidatorNetwork;

use crate::aggregation::registry::ValidatorRegistry;

use super::aggregations::TendermintAggregations;
use super::contribution::TendermintContribution;
use super::utils::{CurrentAggregation, TendermintAggregationEvent};

// Here all the streams are polled to get each and every new aggregate and new round event.
// Special care needs to be taken to release locks as early as possible
pub struct BackgroundTask<N: ValidatorNetwork> {
    aggregations: TendermintAggregations<N>,
    current_aggregate: Arc<RwLock<Option<CurrentAggregation>>>,
    current_bests: Arc<RwLock<BTreeMap<(u32, TendermintStep), TendermintContribution>>>,
    pending_new_round: Arc<RwLock<Option<u32>>>,
    validator_registry: Arc<ValidatorRegistry>,
}

impl<N: ValidatorNetwork + 'static> BackgroundTask<N> {
    pub(super) fn new(
        aggregations: TendermintAggregations<N>,
        current_aggregate: Arc<RwLock<Option<CurrentAggregation>>>,
        current_bests: Arc<RwLock<BTreeMap<(u32, TendermintStep), TendermintContribution>>>,
        pending_new_round: Arc<RwLock<Option<u32>>>,
        validator_registry: Arc<ValidatorRegistry>,
    ) -> Self {
        Self {
            aggregations,
            current_aggregate,
            current_bests,
            pending_new_round,
            validator_registry,
        }
    }

    fn on_new_round(&mut self, next_round: u32) {
        // This needs to be forwarded to a waiting current_aggregate sink if there is one.
        // If it exists, the highest round received in a TendermintAggregationEvent::NewRound(n)
        // must be kept such that it can be send to the next current_aggregate sink if it by then is
        // still relevant.
        let mut aggregate_lock = self.current_aggregate.write();
        let current_aggregate = aggregate_lock.take();

        match &current_aggregate {
            // If there is a current ongoing aggregation it is retrieved.
            Some(CurrentAggregation { sender, round, .. }) => {
                // Check that the next_round is actually higher than the round which is awaiting a result.
                if round < &next_round {
                    // Send the me NewRound event through the channel to the currently awaited
                    // aggregation for it to resolve.
                    if let Err(e) = sender.send(AggregationResult::NewRound(next_round)) {
                        error!(
                            "Sending of AggregationEvent::NewRound({}) failed: {:?}",
                            round, e
                        );
                    }
                } else {
                    // Otherwise this event is meaningless and is dropped, which in reality should
                    // never occur. Log it for debugging purposes.
                    debug!(
                        "Received NewRound({}), but current_aggregate is in round {}",
                        next_round, round
                    );
                    // put the aggregation descriptor back in place to be reused for the next aggregation
                    *aggregate_lock = current_aggregate;
                }
            }
            // If there is no caller awaiting a result
            None => {
                // Advance pending_new_round to next_round if next_round is higher.
                let mut lock = self.pending_new_round.write();
                match lock.as_mut() {
                    Some(current_round) => {
                        *current_round = u32::max(*current_round, next_round);
                    }
                    None => *lock = Some(next_round),
                }
            }
        }
    }

    fn on_aggregation(
        &mut self,
        round: u32,
        step: TendermintStep,
        contribution: TendermintContribution,
    ) {
        trace!("New Aggregate for {}-{:?}: {:?}", round, step, contribution);

        // There is a new aggregate, check if it is a better aggregate for (round, step) than we
        // currently have. If so, store it.
        let contribution = self
            .current_bests
            .write()
            .entry((round, step))
            .and_modify(|contrib| {
                // This test is not strictly necessary as the handel streams will only return a
                // value if it is better than the previous one.
                if self.signature_weight(contrib) < self.signature_weight(&contribution) {
                    *contrib = contribution.clone();
                }
            })
            .or_insert(contribution)
            .clone();

        // Better or not, we need to check if it is actionable for tendermint, and if it is and
        // there is a current_aggregate sink present, we send it as well.
        let lock = self.current_aggregate.read();

        // If there is a current ongoing aggregation it is retrieved
        let current_aggregation = match lock.as_ref() {
            Some(aggregation) => aggregation,
            None => {
                // Do nothing, as there is no caller awaiting a response.
                log::debug!(
                    "There was no current_aggregation for {}-{:?}: {:?}",
                    round,
                    step,
                    contribution
                );
                return;
            }
        };

        // Only reply if it is the correct aggregation and the contribution is actionable.
        // Actionable here means everything with voter_weight >= 2f+1 is potentially actionable.
        // On the receiving end of this channel is a timeout, which will wait (if necessary) for
        // better results but will, if nothing better is made available also return this aggregate.
        if current_aggregation.round == round
            && current_aggregation.step == step
            && self.signature_weight(&contribution) >= policy::TWO_F_PLUS_ONE as usize
        {
            trace!(
                "Completed round for {}-{:?}: {:?}",
                round,
                step,
                contribution
            );

            // Transform to the appropriate result type adding the weight to each proposal's
            // Contribution.
            // TODO optimize
            let result = contribution
                .contributions
                .iter()
                .map(|(hash, contribution)| {
                    (
                        hash.clone(),
                        (
                            contribution.clone(),
                            u16::try_from(self.signature_weight(contribution))
                                .expect("Interger conversion usize -> u16 failed"),
                        ),
                    )
                })
                .collect();

            // Send the result.
            if let Err(err) = current_aggregation
                .sender
                .send(AggregationResult::Aggregation(result))
            {
                error!(
                    "Failed to send message to broadcast_and_aggregate caller: {:?}",
                    err
                );
                // recoverable?
            }
        }
    }

    fn signature_weight<C: AggregatableContribution>(&self, contribution: &C) -> usize {
        self.validator_registry
            .signature_weight(contribution)
            .expect("Failed to compute signature weight")
    }
}

impl<N: ValidatorNetwork + 'static> Future for BackgroundTask<N> {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        loop {
            let event = match ready!(self.aggregations.poll_next_unpin(cx)) {
                Some(event) => event,
                None => {
                    debug!("BackgroundStream finished aggregating - Terminating");
                    return Poll::Ready(());
                }
            };

            match event {
                TendermintAggregationEvent::NewRound(next_round) => self.on_new_round(next_round),
                TendermintAggregationEvent::Aggregation(round, step, contribution) => {
                    self.on_aggregation(round, step, contribution)
                }
            };
        }
    }
}
