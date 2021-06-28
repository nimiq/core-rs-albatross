use nimiq_database::{Database, Environment, Transaction as DBTransaction, WriteTransaction};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::Transaction;

use crate::{AccountError, AccountsTree, Inherent};

pub trait AccountTransactionInteraction: Sized {
    fn create(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        balance: Coin,
        transaction: &Transaction,
        block_height: u32,
        block_time: u64,
    ) -> Result<(), AccountError>;

    fn commit_incoming_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_incoming_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;
}

pub trait AccountInherentInteraction: Sized {
    fn commit_inherent(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_height: u32,
        block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_inherent(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_height: u32,
        block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError>;
}
