use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::sync::mpsc;

use nimiq_block::{MultiSignature, TendermintIdentifier, TendermintStep};
use nimiq_handel::update::LevelUpdate;
use nimiq_handel::update::LevelUpdateMessage;
use nimiq_hash::Blake2sHash;
use nimiq_tendermint::AggregationResult;
use nimiq_validator_network::ValidatorNetwork;

use crate::aggregation::network_sink::NetworkSink;

use super::contribution::TendermintContribution;

/// Struct intended to track the currently awaited round and step of Aggregation.
pub(super) struct CurrentAggregation {
    /// Channel which results are send to
    pub(super) sender: mpsc::UnboundedSender<AggregationResult<Blake2sHash, MultiSignature>>,
    /// The round of the current aggregation
    pub(super) round: u32,
    /// The Step of the current aggregation
    pub(super) step: TendermintStep,
}

/// Struct to describe the different ongoing aggregations
#[derive(std::fmt::Debug)]
pub(super) struct AggregationDescriptor {
    /// Atomic bool keeping track whether the aggregation should continue or not.
    /// once set to false the aggregations next poll call will return Poll::Ready(None)
    /// terminating this aggregation.
    pub(super) is_running: Arc<AtomicBool>,
    /// The sender used for LevelUpdateMessages for this aggregation
    pub(super) input: mpsc::UnboundedSender<LevelUpdate<TendermintContribution>>,
}

/// Internal Wrapper for nimiq_tendermint::AggregationResult. Since the usize indicating the vote weight of each individual signature
/// is only needed in the very end we work on the TendermintContribution in the meantime and convert before returning.
#[derive(std::fmt::Debug)]
pub enum TendermintAggregationEvent {
    /// Indicates updates with a combined vote weight of or exceeding f+1 have been received for a future round (independent of step).
    NewRound(u32),
    /// A new Aggregate(TendermintContribution) is available for a given round(u32) and step(TendermintStep)
    Aggregation(u32, TendermintStep, TendermintContribution),
}

/// Used to pass events from HandelTendermintAdapter to and from TendermintAggregations
pub enum AggregationEvent<N: ValidatorNetwork> {
    Start(
        TendermintIdentifier,
        TendermintContribution,
        Box<NetworkSink<LevelUpdateMessage<TendermintContribution, TendermintIdentifier>, N>>,
    ),
    Cancel(u32, TendermintStep),
}

impl<N: ValidatorNetwork> std::fmt::Debug for AggregationEvent<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AggregationEvent::Start(i, _, _) => f.debug_struct("Start").field("id", i).finish(),
            AggregationEvent::Cancel(r, s) => f.debug_struct("Start").field("id", &(r, s)).finish(),
        }
    }
}
