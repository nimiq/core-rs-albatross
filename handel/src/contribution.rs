use nimiq_collections::bitset::BitSet;
use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum ContributionError {
    #[error("Contributions are overlapping: {0:?}")]
    Overlapping(BitSet),
}

pub trait AggregatableContribution:
    Clone + std::fmt::Debug + Send + Sync + nimiq_serde::Serialize + nimiq_serde::Deserialize + Unpin
{
    /// A BitSet signaling which contributors have contributed in this Contribution
    fn contributors(&self) -> BitSet;

    /// Returns the id of the single contributor
    ///
    /// Panics if there is more than one contributor
    fn contributor(&self) -> usize {
        assert_eq!(self.num_contributors(), 1);
        self.contributors().iter().next().unwrap()
    }

    /// Returns the number of contributions aggregated in this contribution.
    fn num_contributors(&self) -> usize {
        self.contributors().len()
    }

    /// Returns whether the contributors is empty
    fn is_empty(&self) -> bool {
        self.contributors().len() == 0
    }

    /// Combines this contribution with `other_contribution` to create the aggregate of the two.
    ///
    /// The combining contributions must be disjoint.
    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError>;
}
