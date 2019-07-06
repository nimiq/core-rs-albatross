/// TODO: Move to validator/signature_aggregation/config

use std::time::Duration;

use hash::Blake2bHash;
use bls::bls12_381::Signature;


#[derive(Clone, Debug)]
pub struct Config {
    /// Hash of the message that is being signed
    pub message_hash: Blake2bHash,

    /// The signature over `message_hash` that this node contributes
    pub message_signed: Signature,

    /// Number of peers contacted during an update at each level
    pub update_count: usize,

    /// Frequency at which updates are sent to peers
    pub update_period: Duration,

    /// Timeout for levels
    pub timeout: Duration,

    /// How many peers are contacted at each level
    pub peer_count: usize,

}
