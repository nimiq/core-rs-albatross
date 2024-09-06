use std::fmt::Debug;

use nimiq_collections::bitset::BitSet;
use nimiq_serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum ContributionError {
    #[error("Contributions are overlapping: {0:?}")]
    Overlapping(BitSet),
}

pub trait AggregatableContribution:
    Clone + Debug + Send + Sync + Serialize + Deserialize + Unpin
{
    /// A BitSet signaling which contributors have contributed in this Contribution
    fn contributors(&self) -> BitSet;

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
    /// The combining contributions must be disjoint. The original must be retained in case of an error.
    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError>;
}
