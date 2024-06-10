use std::io;

use nimiq_hash::{Blake2sHash, SerializeContent};
use nimiq_serde::{Deserialize, Serialize, SerializedSize};
use serde_repr::{Deserialize_repr, Serialize_repr};

use crate::{
    networks::NetworkId, PREFIX_TENDERMINT_COMMIT, PREFIX_TENDERMINT_PREPARE,
    PREFIX_TENDERMINT_PROPOSAL,
};

/// Internal representation of nimiq_tendermint::Step struct. It needs to be Serializable and must not contain Proposal
/// thus the additional type.
#[derive(
    Serialize_repr, Deserialize_repr, Debug, Clone, Ord, PartialOrd, PartialEq, Eq, Hash, Copy,
)]
#[repr(u8)]
pub enum TendermintStep {
    PreVote = PREFIX_TENDERMINT_PREPARE,
    PreCommit = PREFIX_TENDERMINT_COMMIT,
    Propose = PREFIX_TENDERMINT_PROPOSAL,
}

impl SerializedSize for TendermintStep {
    const SIZE: usize = 1;
}

/// Unique identifier for a single instance of TendermintAggregation
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct TendermintIdentifier {
    /// Network ID this tendermint vote is meant for.
    pub network: NetworkId,
    /// block_number of the to-be-decided-upon macro block.
    pub block_number: u32,
    /// The round number this aggregation accepts contributions for
    pub round_number: u32,
    /// the Step for which contributions are accepted
    pub step: TendermintStep,
}

impl SerializedSize for TendermintIdentifier {
    const SIZE: usize = u8::SIZE + 2 * 4 + TendermintStep::SIZE;
}

// Multiple things this needs to take care of when it comes to what needs signing here:
// First of all to be able to create a block proof the signatures must be over a hash which includes:
// * block-height
// * tendermint round
// * proposal hash (calculated using the `zkp_hash` function)
// * implicit: TendermintStep which also works as the prefix for the specific message which is signed (read purpose byte)
//
// In addition to that the correct assignment of specific contributions to their aggregations also needs part of this information.
// Additionally replay of any given contribution for a different aggregation must not be possible.
// * network
// * block_height
// * round_number
// * step
//
// in summary, the tag which Handel will be working on will be `TendermintIdentifier`
// The signature will then be over the following serialized values (in order):
// `id.step(also prefix) + id.block_number + id.round_number + proposal.header.hash() + create_merkle_root()`
// Note that each one of those is fixed size and thus no overflow from one to the next can be constructed.
//
// The proof needs to contain additional miscellaneous information then, as it would otherwise be lost to time:
// * round_number
//
// that can be included plain text as the proof alongside it also contains it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TendermintVote {
    /// Hash of the proposed macro block
    pub proposal_hash: Option<Blake2sHash>,
    /// Identifier to this votes aggregation
    pub id: TendermintIdentifier,
}

/// Custom Serialize Content, to make sure that
/// * step byte, which is also the message prefix always comes first
/// * options have the same byte length when they are None as when they are Some(x) to prevent overflowing one option into the other.
//
// This needs to be kept in sync with `MacroBlockGadget::tendermint_hash` of
// `nimiq-zkp-circuits`. Whenever this is changed,
// `MacroBlockGadget::tendermint_hash` also needs to be adjusted.
impl SerializeContent for TendermintVote {
    fn serialize_content<W: io::Write, H>(&self, writer: &mut W) -> io::Result<()> {
        // First of all serialize step as this also serves as the unique prefix for this message type.
        self.id.step.serialize_to_writer(writer)?;

        // serialize the network ID
        self.id.network.serialize_to_writer(writer)?;

        // serialize the round number
        self.id
            .round_number
            .to_be_bytes()
            .serialize_to_writer(writer)?;

        // serialize the block number
        self.id
            .block_number
            .to_be_bytes()
            .serialize_to_writer(writer)?;

        // serialize the proposal hash
        self.proposal_hash.serialize_to_writer(writer)?;

        Ok(())
    }
}
