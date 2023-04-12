use std::ops::Deref;

use nimiq_account::{Account, Accounts, BlockLogger, BlockState, Receipts};
use nimiq_database::{volatile::VolatileEnvironment, WriteTransaction};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, key_nibbles::KeyNibbles};
use nimiq_transaction::{inherent::Inherent, Transaction};

pub struct TestCommitRevert {
    accounts: Accounts,
}

impl Default for TestCommitRevert {
    fn default() -> Self {
        Self::new()
    }
}

impl TestCommitRevert {
    pub fn new() -> Self {
        let env = VolatileEnvironment::new(10).unwrap();
        let accounts = Accounts::new(env);
        TestCommitRevert { accounts }
    }

    pub fn with_initial_state(initial_accounts: &[(Address, Account)]) -> Self {
        let test = Self::new();
        let mut txn = WriteTransaction::new(&test.accounts.env);

        for (address, account) in initial_accounts {
            test.accounts
                .tree
                .put(&mut txn, &KeyNibbles::from(address), account)
                .expect("Failed to put initial accounts");
        }

        txn.commit();

        test
    }

    /// Commits the input, then checks that a revert would bring the accounts trie back to the old state.
    /// This keeps the commit part and returns the result of the commit.
    pub fn commit_and_test(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        block_logger: &mut BlockLogger,
    ) -> Result<Receipts, AccountError> {
        self.commit_revert(transactions, inherents, block_state, block_logger, true)
    }

    /// Commits and reverts the input, then checks that the accounts trie is back to the old state.
    /// This doesn't change the state of the accounts. Neither commit nor revert are allowed to fail.
    pub fn test(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
    ) -> Receipts {
        self.commit_revert(
            transactions,
            inherents,
            block_state,
            &mut BlockLogger::empty(),
            false,
        )
        .expect("Failed to commit transactions and inherents")
    }

    fn commit_revert(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        block_logger: &mut BlockLogger,
        keep_commit: bool,
    ) -> Result<Receipts, AccountError> {
        let mut txn = WriteTransaction::new(&self.accounts.env);

        let initial_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        let receipts =
            self.accounts
                .commit(&mut txn, transactions, inherents, block_state, block_logger)?;

        // Potentially keep the commit.
        if keep_commit {
            txn.commit();
            txn = WriteTransaction::new(&self.accounts.env);
        }

        // We only check whether the revert is consistent.
        // We do not do any checks on the intermediate state.

        self.accounts
            .revert(
                &mut txn,
                transactions,
                inherents,
                block_state,
                receipts.clone(),
                &mut BlockLogger::empty(),
            )
            .expect("Failed to revert transactions and inherents");

        let current_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        assert_eq!(initial_root_hash, current_root_hash);

        Ok(receipts)
    }
}

impl Deref for TestCommitRevert {
    type Target = Accounts;

    fn deref(&self) -> &Self::Target {
        &self.accounts
    }
}
