use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::Transaction;

use crate::{AccountError, Inherent};

/// Given the actual sender/receiver account (types), this checks and changes the accounts' states.
/// TODO: `check_incoming_transaction` is not allowed to depend on the account's state.
pub trait AccountTransactionInteraction: Sized {
    fn new_contract(
        account_type: AccountType,
        balance: Coin,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Self, AccountError>;

    fn create(
        balance: Coin,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Self, AccountError>;

    /// Incoming transaction checks must not depend on the account itself and may only be static on the transaction.
    fn check_incoming_transaction(
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<(), AccountError>;

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;

    /// Outgoing transaction checks are usually called from `commit_outgoing_transaction` and may depend on the account's state.
    fn check_outgoing_transaction(
        &self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<(), AccountError>;

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;
}

pub trait AccountInherentInteraction: Sized {
    fn check_inherent(
        &self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
    ) -> Result<(), AccountError>;

    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;
}
