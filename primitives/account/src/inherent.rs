use keys::Address;
use primitives::coin::Coin;

use crate::AccountError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InherentType {
    Reward,
    Slash,
    FinalizeEpoch,
}

impl InherentType {
    /// Inherents can either be applied before transactions in a block or after them.
    /// In most cases, they will be applied after the transactions.
    /// An exception are slash transactions that park a staker.
    /// Following transactions should be able to unpark that staker, which is why slash inherents
    /// are applied before transactions.
    #[inline]
    pub fn is_pre_transactions(&self) -> bool {
        match self {
            InherentType::Slash => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inherent {
    pub ty: InherentType,
    pub target: Address,
    pub value: Coin,
    pub data: Vec<u8>,
}

impl Inherent {
    #[inline]
    pub fn is_pre_transactions(&self) -> bool {
        self.ty.is_pre_transactions()
    }
}

pub trait AccountInherentInteraction: Sized {
    fn check_inherent(&self, inherent: &Inherent, block_height: u32) -> Result<(), AccountError>;

    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;
}
