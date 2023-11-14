use std::time::Duration;

use nimiq_network_interface::request::{MessageMarker, RequestCommon};
use nimiq_tendermint::TaggedAggregationMessage;
use serde::{Deserialize, Serialize};

use super::contribution::AggregateMessage;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TendermintUpdate {
    pub message: TaggedAggregationMessage<AggregateMessage>,
    pub height: u32,
}

impl RequestCommon for TendermintUpdate {
    type Kind = MessageMarker;
    const TYPE_ID: u16 = 124;
    const MAX_REQUESTS: u32 = 500;
    const TIME_WINDOW: std::time::Duration = Duration::from_millis(500);
    type Response = ();
}
