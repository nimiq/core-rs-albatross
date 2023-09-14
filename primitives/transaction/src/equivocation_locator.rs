use nimiq_hash_derive::SerializeContent;
use nimiq_keys::Address;
use nimiq_primitives::TendermintStep;
use nimiq_serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub enum EquivocationLocator {
    Fork(ForkLocator),
    DoubleProposal(DoubleProposalLocator),
    DoubleVote(DoubleVoteLocator),
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub struct ForkLocator {
    pub validator_address: Address,
    pub block_number: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub struct DoubleProposalLocator {
    pub validator_address: Address,
    pub block_number: u32,
    pub round: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub struct DoubleVoteLocator {
    pub validator_address: Address,
    pub block_number: u32,
    pub round: u32,
    pub step: TendermintStep,
}

impl From<ForkLocator> for EquivocationLocator {
    fn from(locator: ForkLocator) -> EquivocationLocator {
        EquivocationLocator::Fork(locator)
    }
}

impl From<DoubleProposalLocator> for EquivocationLocator {
    fn from(locator: DoubleProposalLocator) -> EquivocationLocator {
        EquivocationLocator::DoubleProposal(locator)
    }
}

impl From<DoubleVoteLocator> for EquivocationLocator {
    fn from(locator: DoubleVoteLocator) -> EquivocationLocator {
        EquivocationLocator::DoubleVote(locator)
    }
}
