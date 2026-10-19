use std::time::Duration;

use nimiq_network_interface::request::{MessageMarker, RequestCommon};
use nimiq_tendermint::TaggedAggregationMessage;
use serde::{Deserialize, Serialize};

use crate::aggregation::{
    tendermint::contribution::TendermintContribution, update::SerializableLevelUpdate,
};

/// The message tendermint will use to send a LevelUpdate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TendermintUpdate {
    /// The TaggedAggregationMessage over a SerializableLevelUpdate. The origin is of no concern, as
    /// this will be an outgoing message and the origin will be set by the ValidatorNetwork in its own structure.
    pub message: TaggedAggregationMessage<SerializableLevelUpdate<TendermintContribution>>,
    /// The height this aggregation runs over.
    pub block_number: u32,
}

impl RequestCommon for TendermintUpdate {
    type Kind = MessageMarker;
    type Response = ();
    const TYPE_ID: u16 = 124;
    /// A burst of 500 updates per peer, then one back every millisecond. Honest traffic is a few
    /// updates per 500 ms per peer, so this is generous on purpose: the receiver drops updates
    /// from peers that are not the validator they claim to be before doing any work on them.
    const MAX_REQUESTS: u32 = 500;
    const TIME_WINDOW: Duration = Duration::from_millis(500);
}
