use nimiq_hash_derive::SerializeContent;
use nimiq_keys::Address;
use nimiq_primitives::TendermintStep;
use nimiq_serde::{Deserialize, Serialize};

/// Describes the location of a single equivocation.
///
/// It is used to identify a unique place where an equivocation can happen.
/// E.g. since we don't want to record two fork proofs for the same block
/// height from the same validator, a fork locator consists of a validator
/// address and a block height.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub enum EquivocationLocator {
    /// Fork equivocation.
    Fork(ForkLocator),
    /// Double proposal equivocation.
    DoubleProposal(DoubleProposalLocator),
    /// Double vote equivocation.
    DoubleVote(DoubleVoteLocator),
}

/// Describes the location of a single fork equivocation.
///
/// See [`EquivocationLocator`] for more details.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub struct ForkLocator {
    /// The address of the validator committing the equivocation.
    pub validator_address: Address,
    /// The block height at which the equivocation occured.
    pub block_number: u32,
}

/// Describes the location of a single double proposal equivocation.
///
/// See [`EquivocationLocator`] for more details.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub struct DoubleProposalLocator {
    /// The address of the validator committing the equivocation.
    pub validator_address: Address,
    /// The block height at which the equivocation occured.
    pub block_number: u32,
    /// The round number at which the equivocation occured.
    pub round: u32,
}

/// Describes the location of a single double vote equivocation.
///
/// See [`EquivocationLocator`] for more details.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, SerializeContent)]
pub struct DoubleVoteLocator {
    /// The address of the validator committing the equivocation.
    pub validator_address: Address,
    /// The block height at which the equivocation occured.
    pub block_number: u32,
    /// The round number at which the equivocation occured.
    pub round: u32,
    /// The tendermint step in which the equivocation occured.
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
