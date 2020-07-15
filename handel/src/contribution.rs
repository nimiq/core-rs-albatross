use collections::bitset::BitSet;
use failure::Fail;

#[derive(Clone, Debug, Fail)]
pub enum ContributionError {
    #[fail(display = "Contributions are overlapping: {:?}", _0)]
    Overlapping(BitSet),
}

pub trait Contribution: Clone + std::fmt::Debug {
    fn signers(&self) -> BitSet;
    fn weight(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn add_contribution(&mut self, other_contribution: &Self) -> Result<(), ContributionError>;
}
