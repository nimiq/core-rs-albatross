use serde::{Deserialize, Serialize};

use crate::{
    protocol::{Protocol, SignedProposalMessage},
    state::State,
};

/// The steps of the Tendermint protocol, as described in the paper.
#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum Step {
    #[default]
    Propose = 0,
    Prevote = 1,
    Precommit = 2,
}

impl TryFrom<u8> for Step {
    // Simple Error suffices as there is nothing to differentiate them
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Step::Propose),
            1 => Ok(Step::Prevote),
            2 => Ok(Step::Precommit),
            _ => Err(()),
        }
    }
}

#[derive(Debug)]
pub enum Return<TProtocol: Protocol> {
    Decision(TProtocol::Decision),
    Update(State<TProtocol>),
    ProposalAccepted(SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>),
    ProposalIgnored(SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>),
    ProposalRejected(SignedProposalMessage<TProtocol::Proposal, TProtocol::ProposalSignature>),
}
