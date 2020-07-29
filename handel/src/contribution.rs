use collections::bitset::BitSet;
use failure::Fail;

#[derive(Clone, Debug, Fail)]
pub enum ContributionError {
    #[fail(display = "Contributions are overlapping: {:?}", _0)]
    Overlapping(BitSet),
}

pub trait AggregatableContribution:
    Clone
    + std::fmt::Debug
    + std::marker::Send
    + std::marker::Sync
    + beserial::Serialize
    + beserial::Deserialize
    + Unpin
{
    /// A BitSet signaling which contributors have contributed in this Contribution
    fn contributors(&self) -> BitSet;

    /// returns the id of the single contributor
    ///
    /// panics if there is more than one contributor
    fn contributor(&self) -> usize {
        assert_eq!(self.num_contributors(), 1);
        self.contributors().iter().next().unwrap()
    }

    /// Returns the number of contributions aggregated in this contribution.
    fn num_contributors(&self) -> usize {
        self.contributors().len()
    }

    fn is_empty(&self) -> bool {
        self.contributors().len() == 0
    }

    /// combines this contribution with `other_contribution` to create the aggregate of the two.
    ///
    /// The combining contributions must be disjoint.
    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError>;
}
