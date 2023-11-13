use std::sync::Arc;

use nimiq_block::{MacroBody, MacroHeader};
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_keys::Ed25519Signature as SchnorrSignature;
use nimiq_network_interface::{
    network::Network,
    request::{Handle, RequestCommon, RequestMarker},
};
use nimiq_tendermint::{Inherent, Proposal, ProposalMessage, SignedProposalMessage};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::aggregation::tendermint::state::MacroState;

#[derive(Clone, std::fmt::Debug)]
pub struct Body(pub MacroBody);

impl Inherent<Blake2sHash> for Body {
    fn hash(&self) -> Blake2sHash {
        self.0.hash()
    }
}

/// Used to include a GossipSubId with the proposal. This does not need to serialize or Deserialize as it
/// is not sent over the wire but only used within tendermint.
#[derive(Clone, std::fmt::Debug)]
pub struct Header<Id>(pub MacroHeader, pub Option<Id>);

impl<Id> Proposal<Blake2sHash, Blake2sHash> for Header<Id> {
    fn hash(&self) -> Blake2sHash {
        self.0.hash()
    }

    fn inherent_hash(&self) -> Blake2sHash {
        self.0.body_root.clone()
    }
}

/// This structure represents a proposal for tendermint as it is send over the wire.
/// Tendermint itself does expect a different kind of structure, which will also include
/// Message identifier for GossipSub. That identifier is used to signal the validity of
/// the message to the network. See [nimiq_tendermint::SignedProposalMessage]
#[derive(std::fmt::Debug, Deserialize, Serialize)]
pub struct SignedProposal {
    pub(crate) proposal: MacroHeader,
    pub(crate) round: u32,
    pub(crate) valid_round: Option<u32>,
    pub(crate) signature: SchnorrSignature,
    pub(crate) signer: u16,
}

impl SignedProposal {
    /// Transforms this SignedProposal into a SignedProposalMessage, which tendermint can understand.
    /// Optionally includes the GossipSubId if applicable, or None if the SignedProposal was not received
    /// via GossipSub, i.e. produced by this node itself.
    pub fn into_tendermint_signed_message<Id>(
        self,
        id: Option<Id>,
    ) -> SignedProposalMessage<Header<Id>, (SchnorrSignature, u16)> {
        SignedProposalMessage {
            signature: (self.signature, self.signer),
            message: ProposalMessage {
                proposal: Header(self.proposal, id),
                round: self.round,
                valid_round: self.valid_round,
            },
        }
    }
}

impl<Id> From<SignedProposalMessage<Header<Id>, (SchnorrSignature, u16)>> for SignedProposal {
    fn from(value: SignedProposalMessage<Header<Id>, (SchnorrSignature, u16)>) -> Self {
        Self {
            proposal: value.message.proposal.0,
            valid_round: value.message.valid_round,
            round: value.message.round,
            signature: value.signature.0,
            signer: value.signature.1,
        }
    }
}

pub const MAX_REQUEST_RESPONSE_PROPOSAL: u32 = 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RequestProposal {
    pub block_number: u32,
    pub round_number: u32,
    pub proposal_hash: Blake2sHash,
}

impl RequestCommon for RequestProposal {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 199;
    type Response = Option<SignedProposal>;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_PROPOSAL;
}

impl<N: Network> Handle<N, Option<SignedProposal>, Arc<RwLock<Option<MacroState>>>>
    for RequestProposal
{
    fn handle(
        &self,
        _peer_id: N::PeerId,
        context: &Arc<RwLock<Option<MacroState>>>,
    ) -> Option<SignedProposal> {
        context.read().as_ref().and_then(|state| {
            state.get_proposal_for(self.block_number, self.round_number, &self.proposal_hash)
        })
    }
}
