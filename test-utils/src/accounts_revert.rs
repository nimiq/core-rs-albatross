use std::{fmt::Debug, ops::Deref};

use nimiq_account::{
    Account, AccountInherentInteraction, AccountReceipt, AccountTransactionInteraction, Accounts,
    AccountsError, BlockLog, BlockLogger, BlockState, DataStore, InherentLogger, Receipts,
    TransactionLog,
};
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin, key_nibbles::KeyNibbles};
use nimiq_transaction::{inherent::Inherent, Transaction};
use nimiq_trie::WriteTransactionProxy;
use paste::paste;

macro_rules! impl_accounts_trait {
    ($fn_name: ident, $target: ident, $op_type: ty, $log_type: ty) => {
        paste! {
                pub fn [<test_commit_ $fn_name>]
                <A: AccountTransactionInteraction + AccountInherentInteraction + Clone + Eq + Debug>(
                &self,
                account: &mut A,
                operation: &$op_type,
                block_state: &BlockState,
                logger: &mut $log_type,
                keep_commit: bool,
            ) -> Result<Option<AccountReceipt>, AccountError> {
                let mut raw_txn = self.accounts.env.write_transaction();
                let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
                let initial_account = account.clone();
                let initial_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

                let data_store = DataStore::new(&self.accounts.tree, &operation.$target());
                let receipts = account.[<commit_ $fn_name>](
                    operation,
                    block_state,
                    data_store.write(&mut txn),
                    logger,
                )?;

                // Potentially keep the commit.
                let intermediate_account_state = account.clone();
                if keep_commit {
                    raw_txn.commit();
                    raw_txn = self.accounts.env.write_transaction();
                    txn = (&mut raw_txn).into();
                };

                let mut revert_logger = $log_type::empty();
                account.[<revert_ $fn_name>](
                    operation,
                    block_state,
                    receipts.clone(),
                    data_store.write(&mut txn),
                    &mut revert_logger,
                )?;

                // Check account against initial account after revert.
                assert_eq!(
                    account, &initial_account,
                    "Revert should reset the account state"
                );
                let current_root_hash = self.accounts.get_root_hash_assert(Some(&txn));
                assert_eq!(
                    initial_root_hash, current_root_hash,
                    "Revert should reset the accounts root hash"
                );
                revert_logger.rev_log();

                assert_eq!(&revert_logger, logger, "Logs for committing and reverting should be the same");

                // If we keep the commit, revert account to intermediate state.
                if keep_commit {
                    *account = intermediate_account_state;
                }

                Ok(receipts)
            }
        }
    };
}

pub struct TestCommitRevert {
    accounts: Accounts,
}

impl Default for TestCommitRevert {
    fn default() -> Self {
        Self::new()
    }
}

impl TestCommitRevert {
    impl_accounts_trait!(outgoing_transaction, sender, Transaction, TransactionLog);
    impl_accounts_trait!(incoming_transaction, recipient, Transaction, TransactionLog);
    impl_accounts_trait!(failed_transaction, sender, Transaction, TransactionLog);
    impl_accounts_trait!(inherent, target, Inherent, InherentLogger);

    pub fn new() -> Self {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let accounts = Accounts::new(env);
        TestCommitRevert { accounts }
    }

    pub fn env(&self) -> &MdbxDatabase {
        &self.accounts.env
    }

    pub fn with_initial_state(initial_accounts: &[(Address, Account)]) -> Self {
        let test = Self::new();
        let mut raw_txn = test.accounts.env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        for (address, account) in initial_accounts {
            test.accounts
                .tree
                .put(&mut txn, &KeyNibbles::from(address), account)
                .expect("Failed to put initial accounts");
        }

        raw_txn.commit();

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
    ) -> Result<Receipts, AccountsError> {
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
    ) -> Result<Receipts, AccountsError> {
        let mut raw_txn = self.accounts.env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        let initial_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        let receipts =
            self.accounts
                .commit(&mut txn, transactions, inherents, block_state, block_logger)?;

        // Potentially keep the commit.
        if keep_commit {
            raw_txn.commit();
            raw_txn = self.accounts.env.write_transaction();
            txn = (&mut raw_txn).into();
        }

        // We only check whether the revert is consistent.
        // We do not do any checks on the intermediate state.
        let mut rev_block_logger = BlockLogger::empty_reverted();
        self.accounts
            .revert(
                &mut txn,
                transactions,
                inherents,
                block_state,
                receipts.clone().into(),
                &mut rev_block_logger,
            )
            .expect("Failed to revert transactions and inherents");

        let current_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        assert_eq!(initial_root_hash, current_root_hash);
        let block_log = block_logger.clone().build(0);
        let rev_block_log = rev_block_logger.build(0);

        match (block_log, rev_block_log) {
            (
                BlockLog::AppliedBlock {
                    inherent_logs,
                    tx_logs,
                    ..
                },
                BlockLog::RevertedBlock {
                    inherent_logs: mut rev_inherent_logs,
                    tx_logs: rev_tx_logs,
                    ..
                },
            ) => {
                rev_inherent_logs.reverse();
                assert_eq!(
                    inherent_logs, rev_inherent_logs,
                    "Inherent logs do not match"
                );
                assert_eq!(
                    tx_logs,
                    rev_tx_logs
                        .into_iter()
                        .rev()
                        .map(|mut tx_log| {
                            tx_log.rev_log();
                            tx_log
                        })
                        .collect::<Vec<_>>(),
                    "Tx logs logs do not match"
                );
            }
            _ => panic!("Invalid block logs"),
        }

        Ok(receipts)
    }

    pub fn test_create_new_contract<A: AccountTransactionInteraction + Clone + Eq + Debug>(
        &self,
        transaction: &Transaction,
        initial_balance: Coin,
        block_state: &BlockState,
        tx_logger: &mut TransactionLog,
        keep_commit: bool,
    ) -> Result<Account, AccountError> {
        let mut raw_txn = self.accounts.env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        let initial_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        let data_store = DataStore::new(&self.accounts.tree, &transaction.recipient);
        let mut account = A::create_new_contract(
            transaction,
            initial_balance,
            block_state,
            data_store.write(&mut txn),
            tx_logger,
        )?;

        // Potentially keep the commit.
        let intermediate_account_state = account.clone();
        if keep_commit {
            raw_txn.commit();
            raw_txn = self.accounts.env.write_transaction();
            txn = (&mut raw_txn).into();
        };

        let mut revert_logger = TransactionLog::empty();
        account.revert_new_contract(
            transaction,
            block_state,
            data_store.write(&mut txn),
            &mut revert_logger,
        )?;

        revert_logger.rev_log();

        let current_root_hash = self.accounts.get_root_hash_assert(Some(&txn));
        assert_eq!(
            initial_root_hash, current_root_hash,
            "Revert should reset the accounts root hash"
        );
        assert_eq!(
            &revert_logger, tx_logger,
            "Logs for committing and reverting should be the same"
        );

        Ok(intermediate_account_state)
    }
}

impl Deref for TestCommitRevert {
    type Target = Accounts;

    fn deref(&self) -> &Self::Target {
        &self.accounts
    }
}
