use beserial::{Deserialize, Serialize};
use hash::{Blake2sHash, SerializeContent};
use hash_derive::SerializeContent;
use primitives::slot::ValidatorSlots;

use crate::signed::{Message, SignedMessage, PREFIX_TENDERMINT_PROPOSAL};
use crate::{MacroHeader, MultiSignature};

/// A macro block proposed by the Tendermint leader.
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct TendermintProposal {
    pub value: MacroHeader,
    pub valid_round: Option<u32>,
}

pub type SignedTendermintProposal = SignedMessage<TendermintProposal>;

impl Message for TendermintProposal {
    const PREFIX: u8 = PREFIX_TENDERMINT_PROPOSAL;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct TendermintProof {
    pub round: u32,
    pub sig: MultiSignature,
}

impl TendermintProof {
    pub fn votes(&self) -> u16 {
        // TODO: This probably does not work!!!
        self.sig.signers.len() as u16
    }

    pub fn verify(
        &self,
        block_hash: Blake2sHash,
        block_number: u32,
        validators: &ValidatorSlots,
        threshold: u16,
    ) -> bool {
        if self.votes() < threshold {
            return false;
        }
        true
    }
}
