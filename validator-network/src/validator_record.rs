use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::TaggedSignable;

impl<TPeerId> TaggedSignable for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    const TAG: u8 = 0x03;
}

/// Validator record that is going to be stored into the DHT
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(bound = "TPeerId: Serialize + Deserialize")]
pub struct ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    /// Validator Peer ID
    pub peer_id: TPeerId,
    /// Record timestamp in milliseconds since 1970-01-01 00:00:00 UTC, excluding leap seconds (Unix time)
    pub timestamp: u64,
}

impl<TPeerId> ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub fn new(peer_id: TPeerId, timestamp: u64) -> Self {
        Self { peer_id, timestamp }
    }
}

impl<TPeerId> PartialOrd for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize + PartialEq,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.timestamp.partial_cmp(&other.timestamp)
    }
}

impl<TPeerId> Ord for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize + PartialEq + Eq,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}
