use keys::Address;
use primitives::coin::Coin;

use crate::AccountError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InherentType {
    Reward,
    Slash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inherent {
    pub ty: InherentType,
    pub target: Address,
    pub value: Coin,
    pub data: Vec<u8>,
}

pub trait AccountInherentInteraction: Sized {
    fn check_inherent(&self, inherent: &Inherent) -> Result<(), AccountError>;

    fn commit_inherent(&mut self, inherent: &Inherent) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_inherent(&mut self, inherent: &Inherent, receipt: Option<&Vec<u8>>) -> Result<(), AccountError>;
}

