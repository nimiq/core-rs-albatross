use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::TaggedSignable;

impl<TPeerId> TaggedSignable for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    const TAG: u8 = 0x03;
}

/// Validator record that is going to be stored into the DHT
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(bound = "TPeerId: Serialize + Deserialize")]
pub struct ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    /// Validator Peer ID
    pub peer_id: TPeerId,
}

impl<TPeerId> ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub fn new(peer_id: TPeerId) -> Self {
        Self { peer_id }
    }
}
