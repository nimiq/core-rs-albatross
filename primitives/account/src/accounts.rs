use nimiq_database::{
    traits::{Database, WriteTransaction},
    DatabaseProxy, TransactionProxy as DBTransaction,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::{
    account::{AccountError, AccountType, FailReason},
    key_nibbles::KeyNibbles,
    trie::{
        error::IncompleteTrie,
        trie_chunk::{TrieChunk, TrieChunkPushResult},
        trie_diff::{RevertTrieDiff, TrieDiff},
        trie_proof::TrieProof,
        TrieItem,
    },
    TreeProof,
};
use nimiq_transaction::{inherent::Inherent, ExecutedTransaction, Transaction, TransactionFlags};
use nimiq_trie::{trie::MerkleRadixTrie, WriteTransactionProxy};

use crate::{
    Account, AccountInherentInteraction, AccountPruningInteraction, AccountReceipt,
    AccountTransactionInteraction, BlockLogger, BlockState, DataStore, InherentLogger,
    InherentOperationReceipt, OperationReceipt, Receipts, ReservedBalance, RevertInfo,
    TransactionLog, TransactionOperationReceipt, TransactionReceipt,
};

/// An alias for the accounts tree.
pub type AccountsTrie = MerkleRadixTrie;

/// The Accounts struct is simply an wrapper containing a database environment and, more importantly,
/// a MerkleRadixTrie with accounts as leaf values. This struct basically holds all the accounts in
/// the blockchain. It also has methods to commit and revert transactions, so we can use it to
/// directly update the accounts.
#[derive(Debug)]
pub struct Accounts {
    pub env: DatabaseProxy,
    pub tree: AccountsTrie,
}

impl Accounts {
    /// Creates a new Accounts.
    pub fn new(env: DatabaseProxy) -> Self {
        let tree = AccountsTrie::new(env.clone(), "AccountsTrie");
        Accounts { env, tree }
    }

    /// Initializes the Accounts struct with a given list of accounts.
    pub fn init(&self, txn: &mut WriteTransactionProxy, genesis_accounts: Vec<TrieItem>) {
        self.tree.init(txn, genesis_accounts)
    }

    /// Returns the number of accounts (incl. hybrid nodes) in the Accounts Trie.
    pub fn size(&self) -> u64 {
        let txn = self.env.read_transaction();
        self.tree.num_leaves(&txn) + self.tree.num_hybrids(&txn)
    }

    /// Returns the number of branch nodes in the Accounts Trie.
    pub fn num_branches(&self) -> u64 {
        self.tree.num_branches(&self.env.read_transaction())
    }

    pub fn get(
        &self,
        address: &Address,
        txn_option: Option<&DBTransaction>,
    ) -> Result<Account, IncompleteTrie> {
        let key = KeyNibbles::from(address);
        match txn_option {
            Some(txn) => Ok(self.tree.get(txn, &key)?.unwrap_or_default()),
            None => Ok(self
                .tree
                .get(&self.env.read_transaction(), &key)?
                .unwrap_or_default()),
        }
    }

    pub fn get_complete(&self, address: &Address, txn_option: Option<&DBTransaction>) -> Account {
        self.get(address, txn_option)
            .expect("Tree must be complete")
    }

    /// The given account must correspond to the sender of the given transaction.
    pub fn reserve_balance(
        &self,
        account: &Account,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        block_state: &BlockState,
        txn_option: Option<&DBTransaction>,
    ) -> Result<(), AccountError> {
        // Verify that the account type matches the one given in the transaction.
        if account.account_type() != transaction.sender_type {
            return Err(AccountError::TypeMismatch {
                expected: transaction.sender_type,
                got: account.account_type(),
            });
        }

        // This assumes that the given account corresponds to the sender of the given transaction.
        let store = DataStore::new(&self.tree, &transaction.sender);

        match txn_option {
            Some(txn) => {
                account.reserve_balance(transaction, reserved_balance, block_state, store.read(txn))
            }
            None => account.reserve_balance(
                transaction,
                reserved_balance,
                block_state,
                store.read(&self.env.read_transaction()),
            ),
        }
    }

    /// The given account must correspond to the sender of the given transaction.
    pub fn release_balance(
        &self,
        account: &Account,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        txn_option: Option<&DBTransaction>,
    ) -> Result<(), AccountError> {
        // This assumes that the given account corresponds to the sender of the given transaction.
        let store = DataStore::new(&self.tree, &transaction.sender);

        match txn_option {
            Some(txn) => account.release_balance(transaction, reserved_balance, store.read(txn)),
            None => account.release_balance(
                transaction,
                reserved_balance,
                store.read(&self.env.read_transaction()),
            ),
        }
    }

    pub fn data_store(&self, address: &Address) -> DataStore {
        DataStore::new(&self.tree, address)
    }

    fn get_with_type(
        &self,
        txn: &DBTransaction,
        address: &Address,
        ty: AccountType,
    ) -> Result<Account, AccountError> {
        let account = self.get_complete(address, Some(txn));
        if account.account_type() != ty {
            return Err(AccountError::TypeMismatch {
                expected: ty,
                got: account.account_type(),
            });
        }
        Ok(account)
    }

    fn get_or_restore(
        &self,
        txn: &mut WriteTransactionProxy,
        address: &Address,
        ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
    ) -> Result<Account, AccountError> {
        let account = self
            .tree
            .get::<Account>(txn, &KeyNibbles::from(address))
            .expect("Tree must be complete");
        if let Some(account) = account {
            // TODO This check is unnecessary since we are only using this function during revert.
            //  Replace with assert!()?
            if account.account_type() != ty {
                return Err(AccountError::TypeMismatch {
                    expected: ty,
                    got: account.account_type(),
                });
            }
            Ok(account)
        } else {
            let store = DataStore::new(&self.tree, address);
            Account::restore(ty, pruned_account, store.write(txn))
        }
    }

    fn put(&self, txn: &mut WriteTransactionProxy, address: &Address, account: Account) {
        assert!(!account.can_be_pruned());
        self.tree
            .put(txn, &KeyNibbles::from(address), account)
            .expect("Failed to put account into tree")
    }

    fn put_or_prune(
        &self,
        txn: &mut WriteTransactionProxy,
        address: &Address,
        account: Account,
    ) -> Option<AccountReceipt> {
        if account.can_be_pruned() {
            let store = DataStore::new(&self.tree, address);
            let pruned_account = account.prune(store.read(txn));
            self.prune(txn, address);
            pruned_account
        } else {
            self.put(txn, address, account);
            None
        }
    }

    fn prune(&self, txn: &mut WriteTransactionProxy, address: &Address) {
        // TODO Remove subtree
        self.tree.remove(txn, &KeyNibbles::from(address));
    }

    pub fn get_root_hash_assert(&self, txn_option: Option<&DBTransaction>) -> Blake2bHash {
        match txn_option {
            Some(txn) => self.tree.root_hash_assert(txn),
            None => self.tree.root_hash_assert(&self.env.read_transaction()),
        }
    }

    pub fn get_root_hash(&self, txn_option: Option<&DBTransaction>) -> Option<Blake2bHash> {
        match txn_option {
            Some(txn) => self.tree.root_hash(txn),
            None => self.tree.root_hash(&self.env.read_transaction()),
        }
    }

    pub fn reinitialize_as_incomplete(&self, txn: &mut WriteTransactionProxy) {
        self.tree.reinitialize_as_incomplete(txn)
    }

    pub fn is_complete(&self, txn_option: Option<&DBTransaction>) -> bool {
        match txn_option {
            Some(txn) => self.tree.is_complete(txn),
            None => self.tree.is_complete(&self.env.read_transaction()),
        }
    }

    /// Marks the account at the given address as changed if it is missing.
    /// Returns true if the account is missing, false otherwise.
    fn mark_changed_if_missing(&self, txn: &mut WriteTransactionProxy, address: &Address) -> bool {
        // We consider an account to be missing if the account itself or any of its children are missing.
        // Therefore we need to check if the rightmost child is contained in the missing range.
        let mut rightmost_key = [255u8; KeyNibbles::MAX_BYTES];
        rightmost_key[0..Address::SIZE].copy_from_slice(&address.0);
        let rightmost_key = KeyNibbles::from(&rightmost_key[..]);

        let missing = self
            .tree
            .get_missing_range(txn)
            .map_or(false, |range| range.contains(&rightmost_key));

        if missing {
            self.tree
                .update_within_missing_part(txn, &rightmost_key)
                .expect("Failed to update within missing part");
        }

        missing
    }

    pub fn exercise_transactions(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
    ) -> Result<(Blake2bHash, Blake2bHash, Vec<ExecutedTransaction>), AccountError> {
        let mut raw_txn = self.env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        assert!(self.is_complete(Some(&txn)), "Tree must be complete");

        txn.start_recording();
        let receipts = self.commit(
            &mut txn,
            transactions,
            inherents,
            block_state,
            &mut BlockLogger::empty(),
        )?;
        let diff = txn.stop_recording().into_forward_diff();
        let diff_hash = TreeProof::new(diff.0).root_hash();

        assert_eq!(transactions.len(), receipts.transactions.len());

        let executed_txns = transactions
            .iter()
            .zip(receipts.transactions.iter())
            .map(|(tx, receipt)| match receipt {
                OperationReceipt::Ok(_) => ExecutedTransaction::Ok(tx.clone()),
                OperationReceipt::Err(..) => ExecutedTransaction::Err(tx.clone()),
            })
            .collect();

        let state_hash = self.get_root_hash_assert(Some(&txn));

        raw_txn.abort();

        Ok((state_hash, diff_hash, executed_txns))
    }

    pub fn commit(
        &self,
        txn: &mut WriteTransactionProxy,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        block_logger: &mut BlockLogger,
    ) -> Result<Receipts, AccountError> {
        let receipts =
            self.commit_batch(txn, transactions, inherents, block_state, block_logger)?;
        self.tree.update_root(txn).expect("Tree must be complete");
        Ok(receipts)
    }

    pub fn commit_incomplete(
        &self,
        txn: &mut WriteTransactionProxy,
        diff: TrieDiff,
    ) -> Result<RevertTrieDiff, AccountError> {
        let diff = self.tree.apply_diff(txn, diff)?;
        self.tree.update_root(txn).ok();
        Ok(diff)
    }

    pub fn commit_batch(
        &self,
        txn: &mut WriteTransactionProxy,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        block_logger: &mut BlockLogger,
    ) -> Result<Receipts, AccountError> {
        assert!(self.is_complete(Some(txn)), "Tree must be complete");
        let mut receipts = Receipts::default();

        for transaction in transactions {
            let receipt = self.commit_transaction(
                txn,
                transaction,
                block_state,
                block_logger.new_tx_log(transaction.hash()),
            )?;
            receipts.transactions.push(receipt);
        }

        for inherent in inherents {
            let receipt = self.commit_inherent(
                txn,
                inherent,
                block_state,
                &mut block_logger.inherent_logger(),
            )?;
            receipts.inherents.push(receipt);
        }

        Ok(receipts)
    }

    fn commit_transaction(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        tx_logger: &mut TransactionLog,
    ) -> Result<TransactionOperationReceipt, AccountError> {
        match self.try_commit_transaction(txn, transaction, block_state, tx_logger) {
            Ok(receipt) => Ok(TransactionOperationReceipt::Ok(receipt)),
            Err(e) => {
                let fail_reason = FailReason::from(e);
                tx_logger.clear();
                tx_logger.push_failed_log(transaction, fail_reason);

                let receipt =
                    self.commit_failed_transaction(txn, transaction, block_state, tx_logger)?;
                Ok(TransactionOperationReceipt::Err(receipt, fail_reason))
            }
        }
    }

    /// This function operates atomically i.e. either the transaction is fully committed
    /// (sender + recipient) or it returns an error and no state is changed.
    fn try_commit_transaction(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        tx_logger: &mut TransactionLog,
    ) -> Result<TransactionReceipt, AccountError> {
        // Commit sender.
        let sender_address = &transaction.sender;
        let sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account =
            self.get_with_type(txn, sender_address, transaction.sender_type)?;

        let sender_receipt = sender_account.commit_outgoing_transaction(
            transaction,
            block_state,
            sender_store.write(txn),
            tx_logger,
        )?;

        // Commit recipient.
        let recipient_address = &transaction.recipient;
        let mut recipient_account = Account::default();

        // Handle contract creation.
        let recipient_result = self.commit_recipient(
            txn,
            transaction,
            block_state,
            &mut recipient_account,
            tx_logger,
        );

        // If recipient failed, revert sender.
        if let Err(e) = recipient_result {
            sender_account
                .revert_outgoing_transaction(
                    transaction,
                    block_state,
                    sender_receipt,
                    sender_store.write(txn),
                    tx_logger,
                )
                .expect("failed to revert sender account");

            return Err(e);
        }

        // Update or prune sender.
        let pruned_account = self.put_or_prune(txn, sender_address, sender_account);

        // Update recipient.
        self.put(txn, recipient_address, recipient_account);

        Ok(TransactionReceipt {
            sender_receipt,
            recipient_receipt: recipient_result.unwrap(),
            pruned_account,
        })
    }

    fn commit_recipient(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        recipient_account: &mut Account,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let recipient_address = &transaction.recipient;
        if self.mark_changed_if_missing(txn, recipient_address) {
            return Ok(None);
        }

        let recipient_store = DataStore::new(&self.tree, recipient_address);

        // Handle contract creation.
        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            *recipient_account = self
                .get_with_type(txn, recipient_address, AccountType::Basic)
                .expect("contract creation target must be a basic account");

            Account::create_new_contract(
                transaction,
                recipient_account.balance(),
                block_state,
                recipient_store.write(txn),
                tx_logger,
            )
            .map(|account| {
                *recipient_account = account;
                None
            })
        } else {
            self.get_with_type(txn, recipient_address, transaction.recipient_type)
                .and_then(|account| {
                    *recipient_account = account;
                    recipient_account.commit_incoming_transaction(
                        transaction,
                        block_state,
                        recipient_store.write(txn),
                        tx_logger,
                    )
                })
        }
    }

    fn commit_failed_transaction(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        tx_logger: &mut TransactionLog,
    ) -> Result<TransactionReceipt, AccountError> {
        let sender_address = &transaction.sender;
        if self.mark_changed_if_missing(txn, sender_address) {
            return Ok(TransactionReceipt::default());
        }

        let sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account =
            self.get_with_type(txn, sender_address, transaction.sender_type)?;

        let sender_receipt = sender_account.commit_failed_transaction(
            transaction,
            block_state,
            sender_store.write(txn),
            tx_logger,
        )?;

        let pruned_account = self.put_or_prune(txn, sender_address, sender_account);

        Ok(TransactionReceipt {
            sender_receipt,
            recipient_receipt: None,
            pruned_account,
        })
    }

    fn commit_inherent(
        &self,
        txn: &mut WriteTransactionProxy,
        inherent: &Inherent,
        block_state: &BlockState,
        inherent_logger: &mut InherentLogger,
    ) -> Result<InherentOperationReceipt, AccountError> {
        let address = inherent.target();
        if self.mark_changed_if_missing(txn, address) {
            return Ok(InherentOperationReceipt::Ok(None));
        }

        let store = DataStore::new(&self.tree, address);
        let mut account = self.get_complete(address, Some(txn));

        let result =
            account.commit_inherent(inherent, block_state, store.write(txn), inherent_logger);
        match result {
            Ok(receipt) => {
                self.put(txn, address, account);
                Ok(InherentOperationReceipt::Ok(receipt))
            }
            Err(e) => Ok(InherentOperationReceipt::Err(None, e.into())),
        }
    }

    pub fn revert(
        &self,
        txn: &mut WriteTransactionProxy,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        revert_info: RevertInfo,
        block_logger: &mut BlockLogger,
    ) -> Result<(), AccountError> {
        match revert_info {
            RevertInfo::Receipts(receipts) => {
                self.revert_batch(
                    txn,
                    transactions,
                    inherents,
                    block_state,
                    receipts,
                    block_logger,
                )?;
            }
            RevertInfo::Diff(diff) => {
                self.revert_diff(txn, diff)?;
            }
        }
        self.tree.update_root(txn).ok();
        Ok(())
    }

    pub fn revert_diff(
        &self,
        txn: &mut WriteTransactionProxy,
        diff: RevertTrieDiff,
    ) -> Result<(), AccountError> {
        self.tree.revert_diff(txn, diff)?;
        Ok(())
    }

    pub fn revert_batch(
        &self,
        txn: &mut WriteTransactionProxy,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        receipts: Receipts,
        block_logger: &mut BlockLogger,
    ) -> Result<(), AccountError> {
        // Revert inherents in reverse order.
        assert_eq!(inherents.len(), receipts.inherents.len());
        let iter = inherents.iter().zip(receipts.inherents).rev();
        for (inherent, receipt) in iter {
            self.revert_inherent(
                txn,
                inherent,
                block_state,
                receipt,
                &mut block_logger.inherent_logger(),
            )?;
        }

        // Revert transactions in reverse order.
        assert_eq!(transactions.len(), receipts.transactions.len());
        let iter = transactions.iter().zip(receipts.transactions).rev();
        for (transaction, receipt) in iter {
            self.revert_transaction(
                txn,
                transaction,
                block_state,
                receipt,
                block_logger.new_tx_log(transaction.hash()),
            )?;
        }

        Ok(())
    }

    fn revert_transaction(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        receipt: TransactionOperationReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        match receipt {
            OperationReceipt::Ok(receipt) => self.revert_successful_transaction(
                txn,
                transaction,
                block_state,
                receipt,
                tx_logger,
            ),
            OperationReceipt::Err(receipt, fail_reason) => {
                let result = self.revert_failed_transaction(
                    txn,
                    transaction,
                    block_state,
                    receipt,
                    tx_logger,
                );
                tx_logger.push_failed_log(transaction, fail_reason);

                result
            }
        }
    }

    /// FIXME This function might leave the WriteTransactionProxy in an inconsistent state if it returns an error!
    fn revert_successful_transaction(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        receipt: TransactionReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Revert recipient first.
        let recipient_address = &transaction.recipient;
        if !self.mark_changed_if_missing(txn, recipient_address) {
            let recipient_store = DataStore::new(&self.tree, recipient_address);
            let mut recipient_account =
                self.get_with_type(txn, recipient_address, transaction.recipient_type)?;

            // Revert contract creation.
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                recipient_account.revert_new_contract(
                    transaction,
                    block_state,
                    recipient_store.write(txn),
                    tx_logger,
                )?;

                recipient_account = Account::default_with_balance(recipient_account.balance());
            } else {
                recipient_account.revert_incoming_transaction(
                    transaction,
                    block_state,
                    receipt.recipient_receipt,
                    recipient_store.write(txn),
                    tx_logger,
                )?;
            }

            // Update or prune recipient.
            // The recipient account might have been created by the incoming transaction.
            self.put_or_prune(txn, recipient_address, recipient_account);
        }

        // Revert sender. It might need to be restored first if it was pruned.
        let sender_address = &transaction.sender;
        if !self.mark_changed_if_missing(txn, sender_address) {
            let sender_store = DataStore::new(&self.tree, sender_address);
            let mut sender_account = self.get_or_restore(
                txn,
                sender_address,
                transaction.sender_type,
                receipt.pruned_account.as_ref(),
            )?;

            sender_account.revert_outgoing_transaction(
                transaction,
                block_state,
                receipt.sender_receipt,
                sender_store.write(txn),
                tx_logger,
            )?;

            // Store sender.
            // Reverting a zero-fee signaling transaction can create a prunable account.
            self.put_or_prune(txn, sender_address, sender_account);
        }

        Ok(())
    }

    fn revert_failed_transaction(
        &self,
        txn: &mut WriteTransactionProxy,
        transaction: &Transaction,
        block_state: &BlockState,
        receipt: TransactionReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let sender_address = &transaction.sender;
        if self.mark_changed_if_missing(txn, sender_address) {
            return Ok(());
        }

        let sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account = self.get_or_restore(
            txn,
            sender_address,
            transaction.sender_type,
            receipt.pruned_account.as_ref(),
        )?;

        sender_account.revert_failed_transaction(
            transaction,
            block_state,
            receipt.sender_receipt,
            sender_store.write(txn),
            tx_logger,
        )?;

        // Reverting a zero-fee signaling transaction can create a prunable account.
        self.put_or_prune(txn, sender_address, sender_account);

        Ok(())
    }

    fn revert_inherent(
        &self,
        txn: &mut WriteTransactionProxy,
        inherent: &Inherent,
        block_state: &BlockState,
        receipt: InherentOperationReceipt,
        inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        // If the inherent operation failed, there is nothing to revert.
        let receipt = match receipt {
            OperationReceipt::Ok(receipt) => receipt,
            OperationReceipt::Err(..) => return Ok(()),
        };

        let address = inherent.target();
        if self.mark_changed_if_missing(txn, address) {
            return Ok(());
        }

        let store = DataStore::new(&self.tree, address);
        let mut account = self.get_complete(address, Some(txn));

        account.revert_inherent(
            inherent,
            block_state,
            receipt,
            store.write(txn),
            inherent_logger,
        )?;

        // The account might have been created by the inherent (i.e. reward inherent).
        self.put_or_prune(txn, address, account);

        Ok(())
    }

    pub fn finalize_batch(&self, txn: &mut WriteTransactionProxy) {
        // It is fine to have an incomplete trie here.
        self.tree.update_root(txn).ok();
    }

    pub fn commit_chunk(
        &self,
        txn: &mut WriteTransactionProxy,
        chunk: TrieChunk,
        expected_hash: Blake2bHash,
        start_key: KeyNibbles,
    ) -> Result<TrieChunkPushResult, AccountError> {
        self.tree
            .put_chunk(txn, start_key, chunk, expected_hash)
            .map_err(AccountError::from)
    }

    pub fn revert_chunk(
        &self,
        txn: &mut WriteTransactionProxy,
        start_key: KeyNibbles,
    ) -> Result<(), AccountError> {
        self.tree.remove_chunk(txn, start_key)?;
        Ok(())
    }

    pub fn get_chunk(
        &self,
        start_key: KeyNibbles,
        limit: usize,
        txn_option: Option<&DBTransaction>,
    ) -> TrieChunk {
        match txn_option {
            Some(txn) => self.tree.get_chunk_with_proof(txn, start_key.., limit),
            None => {
                self.tree
                    .get_chunk_with_proof(&self.env.read_transaction(), start_key.., limit)
            }
        }
    }

    /// Produces a Merkle proof of the inclusion of the given keys in the
    /// Merkle Radix Trie.
    pub fn get_proof(
        &self,
        txn_option: Option<&DBTransaction>,
        keys: Vec<&KeyNibbles>,
    ) -> Result<TrieProof, IncompleteTrie> {
        match txn_option {
            Some(txn) => self.tree.get_proof(txn, keys),
            None => self.tree.get_proof(&self.env.read_transaction(), keys),
        }
    }
}
