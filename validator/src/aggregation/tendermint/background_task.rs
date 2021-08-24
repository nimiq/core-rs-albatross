use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::task::Poll;

use futures::{Future, StreamExt};

use nimiq_block::TendermintStep;
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
}

impl<N: ValidatorNetwork + 'static> Future for BackgroundTask<N> {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        match self.aggregations.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                debug!("BackgroundStream finished aggregating - Terminating");
                Poll::Ready(())
            }
            Poll::Ready(Some(event)) => {
                match event {
                    TendermintAggregationEvent::NewRound(next_round) => {
                        // This needs to be forwarded to a waiting current_aggregate sink if there is one.
                        // if there is none, the highest round received in a TendermintAggregationEvent::NewRound(n) must be kept
                        // such that it can be send to the next current_aggregate sink if it by then is still relevant.
                        match self
                            .current_aggregate
                            .write()
                            .expect("current_aggregate lock could not be aquired.")
                            .take()
                        {
                            // if there is a current ongoing aggregation it is retrieved
                            Some(CurrentAggregation {
                                sender,
                                round,
                                step: _step,
                            }) => {
                                // check that the next_round is actually higher than the round which is awaiting a result.
                                if round < next_round {
                                    // Send the me NewRound event through the channel to the currently awaited aggregation for it to resolve.
                                    if let Err(_err) =
                                        sender.send(AggregationResult::NewRound(next_round))
                                    {
                                        error!(
                                            "Sending of AggregationEvent::NewRound({}) failed",
                                            round
                                        );
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
                                let mut lock = self
                                    .pending_new_round
                                    .write()
                                    .expect("pending_new_round lock couuld not be aquired.");

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
                        trace!("New Aggregate for {}-{:?}: {:?}", &r, &s, &c);
                        // There is a new aggregate, check if it is a better aggregate for (round, step) than we curretly have
                        // if so, store it.
                        let contribution = self
                            .current_bests
                            .write()
                            .expect("current_bests lock could not be aquired.")
                            .entry((r, s))
                            .and_modify(|contribution| {
                                // this test is not strictly necessary as the handel streams will only return a value if it is better  than the previous one.
                                if self
                                    .validator_registry
                                    .signature_weight(contribution)
                                    .expect("Failed to unwrap signature weight")
                                    < self
                                        .validator_registry
                                        .signature_weight(&c)
                                        .expect("Failed to unwrap signature weight")
                                {
                                    *contribution = c.clone();
                                }
                            })
                            .or_insert(c)
                            .clone();

                        // better or not, we need to check if it is actionable for tendermint, and if it is and there is a
                        // current_aggregate sink present, we send it as well.
                        let mut lock = self
                            .current_aggregate
                            .write()
                            .expect("current_aggregate lock could not be aquired.");

                        match lock.take() {
                            // if there is a current ongoing aggregation it is retrieved
                            Some(CurrentAggregation {
                                sender,
                                round,
                                step,
                            }) => {
                                // only reply if it is the correct aggregation
                                if round == r && step == s {
                                    // see if this is actionable
                                    // Actionable here means everything with voter_weight > 2f+1 is potentially actionable.
                                    // On the receiving end of this channel is a timeout, which will wait (if necessary) for better results
                                    // but will, if nothing better is made available also return this aggregate.
                                    if self
                                        .validator_registry
                                        .signature_weight(&contribution)
                                        .expect("Failed to unwrap signature weight")
                                        > policy::TWO_THIRD_SLOTS as usize
                                    {
                                        trace!(
                                            "Completed Round for {}-{:?}: {:?}",
                                            &r,
                                            &s,
                                            &contribution
                                        );
                                        // Transform to the appropiate result type adding the weight to each proposals Contribution
                                        let result = contribution
                                            .contributions
                                            .iter()
                                            .map(|(hash, contribution)| {
                                                (
                                                    hash.clone(),
                                                    (
                                                        contribution.clone(),
                                                        self.validator_registry.signature_weight(contribution).expect("Cannot compute weight of signatories"),
                                                    ),
                                                )
                                            })
                                            .collect();

                                        // send the result
                                        if let Err(err) =
                                            sender.send(AggregationResult::Aggregation(result))
                                        {
                                            error!("failed sending message to broadcast_and_aggregate caller: {:?}", err);
                                            // recoverable?
                                        }
                                    } else {
                                        // re-set the current_aggregate
                                        *lock = Some(CurrentAggregation {
                                            sender,
                                            round,
                                            step,
                                        });
                                    }
                                } else {
                                    // re-set the current_aggregate
                                    *lock = Some(CurrentAggregation {
                                        sender,
                                        round,
                                        step,
                                    });
                                }
                            }
                            None => {
                                debug!("there was no receiver");
                                // do nothing, as there is no caller awaiting a response.
                            }
                        };
                    }
                };

                Poll::Pending
            }
        }
    }
}
