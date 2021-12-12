use log::error;
use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_bls::AggregatePublicKey;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, SerializeContent};
use nimiq_hash_derive::SerializeContent;
use nimiq_primitives::policy::TWO_THIRD_SLOTS;
use nimiq_primitives::slots::Validators;

use crate::signed::{
    Message, SignedMessage, PREFIX_TENDERMINT_COMMIT, PREFIX_TENDERMINT_PREPARE,
    PREFIX_TENDERMINT_PROPOSAL,
};
use crate::{MacroBlock, MacroHeader, MultiSignature};

/// The proposal message sent by the Tendermint leader.
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct TendermintProposal {
    /// The header of the macro block, which is effectively the proposal.
    pub value: MacroHeader,
    /// The valid round of the proposer. See the Tendermint crate for more details.
    pub valid_round: Option<u32>,
    /// the round this proposal was created for
    pub round: u32,
}

impl Message for TendermintProposal {
    const PREFIX: u8 = PREFIX_TENDERMINT_PROPOSAL;
}

pub type SignedTendermintProposal = SignedMessage<TendermintProposal>;

/// The proof for a block produced by Tendermint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct TendermintProof {
    // The round when the block was completed. This is necessary to verify the signature.
    pub round: u32,
    // The aggregated signature of the precommits of the validators for this block.
    pub sig: MultiSignature,
}

impl TendermintProof {
    /// This simply returns the number of slots that voted (precommitted) to this block.
    pub fn votes(&self) -> u16 {
        self.sig.signers.len() as u16
    }

    /// Verifies the proof. This only checks that the proof is valid for this block, not that the
    /// block itself is valid.
    pub fn verify(block: &MacroBlock, current_validators: &Validators) -> bool {
        // If there's no justification then the proof is false evidently.
        let justification = match &block.justification {
            None => {
                error!("Invalid justification - macro block has no justification!");
                return false;
            }
            Some(x) => x,
        };

        // Check if there are enough votes.
        if justification.votes() < TWO_THIRD_SLOTS {
            error!("Invalid justification - not enough votes!");
            return false;
        }

        // Calculate the `nano_zkp_hash`. This a special hash that is calculated using the `validators`
        // field of the block body. It is necessary for the ZKP proofs used in the nano sync.
        let block_hash = block.nano_zkp_hash();

        // Calculate the message that was actually signed by the validators.
        let message = TendermintVote {
            proposal_hash: Some(block_hash),
            id: TendermintIdentifier {
                block_number: block.block_number(),
                round_number: justification.round,
                step: TendermintStep::PreCommit,
            },
        };

        // Get the public key for each SLOT and add them together to get the aggregated public key
        // (if they are part of the Multisignature Bitset).
        let mut agg_pk = AggregatePublicKey::new();

        for (i, pk) in current_validators.voting_keys().iter().enumerate() {
            if justification.sig.signers.contains(i as usize) {
                agg_pk.aggregate(pk);
            }
        }

        // Verify the aggregated signature against our aggregated public key.
        agg_pk.verify(&message, &justification.sig.signature)
    }
}

/// Internal representation of nimiq_tendermint::Step struct. It needs to be Serializable and must not contain Proposal
/// thus the additional type.
#[derive(Serialize, Deserialize, Debug, Clone, Ord, PartialOrd, PartialEq, Eq, Hash, Copy)]
#[repr(u8)]
pub enum TendermintStep {
    PreVote = PREFIX_TENDERMINT_PREPARE,
    PreCommit = PREFIX_TENDERMINT_COMMIT,
    Propose = 0x77,
}

/// Unique identifier for a single instance of TendermintAggregation
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct TendermintIdentifier {
    /// block_number of the to-be-decided-upon macro block.
    pub block_number: u32,
    /// The round number this aggregation accepts contributions for
    pub round_number: u32,
    /// the Step for which contributions are accepted
    pub step: TendermintStep,
}

// Multiple things this needs to take care of when it comes to what needs signing here:
// First of all to be able to create a block proof the signatures must be over a hash which includes:
// * block-height
// * tendermint round
// * proposal hash (calculated using the `nano_zkp_hash` function)
// * implicit: TendermintStep which also works as the prefix for the specific message which is signed (read purpose byte)
//
// In addition to that the correct assignment of specific contributions to their aggregations also needs part of this information.
// Additionally replay of any given contribution for a different aggregation must not be possible.
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
impl SerializeContent for TendermintVote {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        // First of all serialize step as this also serves as the unique prefix for this message type.
        let mut size = self.id.step.serialize(writer)?;

        // serialize the round number
        size += self.id.round_number.serialize(writer)?;

        // serialize the block number
        size += self.id.block_number.serialize(writer)?;

        // For the hash, make sure that if the Option is None the byte length stays the same, just filled with 0s.
        // Also have a byte to indicate if it is a None or a Some.
        match &self.proposal_hash {
            Some(hash) => {
                size += true.serialize(writer)?;

                size += hash.serialize(writer)?;
            }
            None => {
                size += false.serialize(writer)?;

                let zero_bytes: Vec<u8> = vec![0u8, Blake2bHash::SIZE as u8];
                writer.write_all(zero_bytes.as_slice())?;
                size += Blake2bHash::SIZE;
            }
        };

        // Finally attempt to flush
        writer.flush()?;

        // And return the size
        Ok(size)
    }
}

impl Hash for TendermintVote {}
