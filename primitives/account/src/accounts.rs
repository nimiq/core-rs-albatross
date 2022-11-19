use nimiq_database::{
    Environment, ReadTransaction, Transaction as DBTransaction, WriteTransaction,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::{
    account::AccountError,
    key_nibbles::KeyNibbles,
    trie::trie_chunk::{TrieChunk, TrieChunkPushResult},
};
use nimiq_transaction::{inherent::Inherent, ExecutedTransaction, Transaction, TransactionFlags};
use nimiq_trie::trie::{IncompleteTrie, MerkleRadixTrie};

use crate::data_store::DataStore;
use crate::interaction_traits::AccountPruningInteraction;
use crate::{
    Account, AccountInherentInteraction, AccountReceipt, AccountTransactionInteraction, Inherent,
    InherentOperationReceipt, OperationReceipt, Receipts, TransactionOperationReceipt,
    TransactionReceipt,
};

/// An alias for the accounts tree.
pub type AccountsTrie = MerkleRadixTrie;

/// The Accounts struct is simply an wrapper containing a database environment and, more importantly,
/// a MerkleRadixTrie with accounts as leaf values. This struct basically holds all the accounts in
/// the blockchain. It also has methods to commit and revert transactions, so we can use it to
/// directly update the accounts.
#[derive(Debug)]
pub struct Accounts {
    pub env: Environment,
    pub tree: AccountsTrie,
}

impl Accounts {
    /// Creates a new Accounts.
    pub fn new(env: Environment) -> Self {
        let tree = AccountsTrie::new(env.clone(), "AccountsTrie");
        Accounts { env, tree }
    }

    /// Initializes the Accounts struct with a given list of accounts.
    pub fn init(&self, txn: &mut WriteTransaction, genesis_accounts: Vec<(KeyNibbles, Account)>) {
        for (key, account) in genesis_accounts {
            self.tree
                .put(txn, &key, account)
                .expect("temporary until accounts rewrite");
        }
        self.tree
            .update_root(txn)
            .expect("should be a complete trie");
    }

    /// Returns the number of accounts (incl. hybrid nodes) in the Accounts Trie.
    pub fn size(&self) -> u64 {
        let txn = ReadTransaction::new(&self.env);
        self.tree.num_leaves(&txn) + self.tree.num_hybrids(&txn)
    }

    /// Returns the number of branch nodes in the Accounts Trie.
    pub fn num_branches(&self) -> u64 {
        self.tree.num_branches(&ReadTransaction::new(&self.env))
    }

    pub fn get(
        &self,
        address: &Address,
        txn_option: Option<&DBTransaction>,
    ) -> Result<Option<Account>, IncompleteTrie> {
        let key = KeyNibbles::from(address);
        match txn_option {
            Some(txn) => self.tree.get(txn, &key),
            None => self.tree.get(&ReadTransaction::new(&self.env), &key),
        }
    }

    fn get_with_type(
        &self,
        txn: &DBTransaction,
        address: &Address,
        ty: AccountType,
    ) -> Result<Account, AccountError> {
        let account = self.get(address, Some(txn))?;
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
        txn: &mut WriteTransaction,
        address: &Address,
        ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
    ) -> Result<Account, AccountError> {
        if let Some(account) = self.tree.get(txn, &KeyNibbles::from(address)) {
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

    fn put(&self, txn: &mut WriteTransaction, address: &Address, account: Account) {
        self.tree.put(txn, &KeyNibbles::from(address), account)
    }

    fn put_or_prune(
        &self,
        txn: &mut WriteTransaction,
        address: &Address,
        account: Account,
    ) -> Option<AccountReceipt> {
        if account.can_be_pruned() {
            let store = DataStore::new(&self.tree, address);
            let pruned_account = account.prune(store.read(txn))?;
            self.prune(txn, address);
            pruned_account
        } else {
            self.put(txn, address, account);
            None
        }
    }

    fn prune(&self, txn: &mut WriteTransaction, address: &Address) {
        // TODO Remove subtree
        self.tree.remove(txn, &KeyNibbles::from(address));
    }

    pub fn get_root_hash_assert(&self, txn_option: Option<&DBTransaction>) -> Blake2bHash {
        match txn_option {
            Some(txn) => self.tree.root_hash_assert(txn),
            None => self.tree.root_hash_assert(&ReadTransaction::new(&self.env)),
        }
    }

    pub fn get_root_hash(&self, txn_option: Option<&DBTransaction>) -> Option<Blake2bHash> {
        match txn_option {
            Some(txn) => self.tree.root_hash(txn),
            None => self.tree.root_hash(&ReadTransaction::new(&self.env)),
        }
    }

    pub fn reinitialize_as_incomplete(&self, txn: &mut WriteTransaction) {
        self.tree.reinitialize_as_incomplete(txn)
    }

    pub fn is_complete(&self, txn_option: Option<&DBTransaction>) -> bool {
        match txn_option {
            Some(txn) => self.tree.is_complete(txn),
            None => self.tree.is_complete(&ReadTransaction::new(&self.env)),
        }
    }

    pub fn exercise_transactions(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<(Blake2bHash, Vec<ExecutedTransaction>), AccountError> {
        todo!()
        // let mut txn = WriteTransaction::new(&self.env);
        //
        // let (_, executed_txns) =
        //     self.commit(&mut txn, transactions, inherents, block_height, timestamp)?;
        //
        // let hash = self.root_hash(Some(&txn));
        //
        // txn.abort();
        //
        // Ok((hash, executed_txns))
    }

    pub fn commit(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_time: u64,
    ) -> Result<Receipts, AccountError> {
        let receipts = self.commit_batch(txn, transactions, inherents, block_time)?;
        self.tree.update_root(txn);
        Ok(receipts)
    }

    pub fn commit_batch(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_time: u64,
    ) -> Result<Receipts, AccountError> {
        let mut receipts = Receipts::default();

        for transaction in transactions {
            let receipt = self.commit_transaction(txn, transaction, block_time)?;
            receipts.transactions.push(receipt);
        }

        for inherent in inherents {
            let receipt = self.commit_inherent(txn, inherent, block_time)?;
            receipts.inherents.push(receipt);
        }

        Ok(receipts)
    }

    fn commit_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_time: u64,
    ) -> Result<TransactionOperationReceipt, AccountError> {
        if let Ok(receipt) = self.try_commit_transaction(txn, transaction, block_time) {
            Ok(TransactionOperationReceipt::Ok(receipt))
        } else {
            let receipt = self.commit_failed_transaction(txn, transaction, block_time)?;
            Ok(TransactionOperationReceipt::Err(receipt))
        }
    }

    /// This function operates atomically i.e. either the transaction is fully committed
    /// (sender + recipient) or it returns an error and no state is changed.
    fn try_commit_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_time: u64,
    ) -> Result<TransactionReceipt, AccountError> {
        // Commit sender.
        let sender_address = &transaction.sender;
        let mut sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account =
            self.get_with_type(txn, sender_address, transaction.sender_type)?;

        let mut sender_receipt = sender_account.commit_outgoing_transaction(
            transaction,
            block_time,
            sender_store.write(txn),
        )?;

        // Commit recipient.
        let recipient_address = &transaction.recipient;
        let mut recipient_store = DataStore::new(&self.tree, recipient_address);
        let mut recipient_account = Account::default();

        // Handle contract creation.
        let recipient_result = if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            recipient_account = self
                .get_with_type(txn, recipient_address, AccountType::Basic)
                .expect("contract creation target must be a basic account");

            Account::create_new_contract(
                transaction,
                recipient_account.balance(),
                block_time,
                recipient_store.write(txn),
            )
            .map(|account| {
                recipient_account = account;
                None
            })
        } else {
            self.get_with_type(txn, recipient_address, transaction.recipient_type)
                .and_then(|mut account| {
                    recipient_account = account;
                    recipient_account.commit_incoming_transaction(
                        transaction,
                        block_time,
                        recipient_store.write(txn),
                    )
                })
        };

        // If recipient failed, revert sender.
        if let Err(e) = recipient_result {
            // TODO Log error

            sender_account
                .revert_outgoing_transaction(
                    transaction,
                    block_time,
                    sender_receipt.as_ref(),
                    sender_store.write(txn),
                )
                .expect("failed to revert sender account");

            return Err(e);
        }

        // Update or prune sender.
        let pruned_account = self.put_or_prune(txn, sender_address, sender_account);

        // Update recipient.
        assert!(!recipient_account.can_be_pruned());
        self.put(txn, recipient_address, recipient_account);

        Ok(TransactionReceipt {
            sender_receipt,
            recipient_receipt: recipient_result.unwrap(),
            pruned_account,
        })
    }

    fn commit_failed_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_time: u64,
    ) -> Result<TransactionReceipt, AccountError> {
        let sender_address = &transaction.sender;
        let mut sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account =
            self.get_with_type(txn, sender_address, transaction.sender_type)?;

        let sender_receipt = sender_account.commit_failed_transaction(
            transaction,
            block_time,
            sender_store.write(txn),
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
        txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_time: u64,
    ) -> Result<InherentOperationReceipt, AccountError> {
        let address = inherent.target();
        let store = DataStore::new(&self.tree, address);
        let mut account = self.get(address, Some(txn))?;

        if let Ok(receipt) = account.commit_inherent(inherent, block_time, store.write(txn)) {
            self.put(txn, address, account);
            Ok(InherentOperationReceipt::Ok(receipt))
        } else {
            // TODO Log error
            Ok(InherentOperationReceipt::Err(None))
        }
    }

    pub fn revert(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_time: u64,
        receipts: &Receipts,
    ) -> Result<(), AccountError> {
        self.revert_batch(txn, transactions, inherents, block_time, receipts)?;
        // It is fine to have an incomplete trie here.
        let _ = self.tree.update_root(txn);
        Ok(())
    }

    pub fn revert_batch(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_time: u64,
        receipts: &Receipts,
    ) -> Result<(), AccountError> {
        // Revert inherents in reverse order.
        assert_eq!(inherents.len(), receipts.inherents.len());
        let iter = inherents.into_iter().zip(receipts.inherents.iter()).rev();
        for (inherent, receipt) in iter {
            self.revert_inherent(txn, inherent, block_time, receipt)?;
        }

        // Revert transactions in reverse order.
        assert_eq!(transactions.len(), receipts.transactions.len());
        let iter = transactions
            .into_iter()
            .zip(receipts.transactions.iter())
            .rev();
        for (transaction, receipt) in iter {
            self.revert_transaction(txn, transaction, block_time, receipt)?;
        }

        Ok(())
    }

    fn revert_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_time: u64,
        receipt: &TransactionOperationReceipt,
    ) -> Result<(), AccountError> {
        match receipt {
            OperationReceipt::Ok(receipt) => {
                self.revert_successful_transaction(txn, transaction, block_time, receipt)
            }
            OperationReceipt::Err(receipt) => {
                self.revert_failed_transaction(txn, transaction, block_time, receipt)
            }
        }
    }

    /// TODO This function might leave the WriteTransaction in an inconsistent state if it returns an error!
    fn revert_successful_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_time: u64,
        receipt: &TransactionReceipt,
    ) -> Result<(), AccountError> {
        // Revert recipient first.
        let recipient_address = &transaction.recipient;
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
                block_time,
                recipient_store.write(txn),
            )?;

            recipient_account = Account::default_with_balance(recipient_account.balance());
        } else {
            recipient_account.revert_incoming_transaction(
                transaction,
                block_time,
                receipt.recipient_receipt.as_ref(),
                recipient_store.write(txn),
            )?;
        }

        // Revert sender. It might need to be restored first if it was pruned.
        let sender_address = &transaction.sender;
        let sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account = self.get_or_restore(
            txn,
            sender_address,
            transaction.sender_type,
            receipt.pruned_account.as_ref(),
        )?;

        sender_account.revert_outgoing_transaction(
            transaction,
            block_time,
            receipt.sender_receipt.as_ref(),
            sender_store.write(txn),
        )?;

        // Store sender.
        self.put(txn, sender_address, sender_account);

        // Update or prune recipient.
        // The recipient account might have been created by the incoming transaction.
        self.put_or_prune(txn, recipient_address, recipient_account);

        Ok(())
    }

    fn revert_failed_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_time: u64,
        receipt: &TransactionReceipt,
    ) -> Result<(), AccountError> {
        let sender_address = &transaction.sender;
        let mut sender_store = DataStore::new(&self.tree, sender_address);
        let mut sender_account = self.get_or_restore(
            txn,
            sender_address,
            transaction.sender_type,
            receipt.pruned_account.as_ref(),
        )?;

        sender_account.revert_failed_transaction(
            transaction,
            block_time,
            receipt.sender_receipt.as_ref(),
            sender_store.write(txn),
        )?;

        self.put(txn, sender_address, sender_account);

        Ok(())
    }

    fn revert_inherent(
        &self,
        txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_time: u64,
        receipt: &InherentOperationReceipt,
    ) -> Result<(), AccountError> {
        // If the inherent operation failed, there is nothing to revert.
        let receipt = match receipt {
            OperationReceipt::Ok(receipt) => receipt,
            OperationReceipt::Err(_) => return Ok(()),
        };

        let address = inherent.target();
        let store = DataStore::new(&self.tree, address);
        let mut account = self.get(address, Some(txn))?;

        account.revert_inherent(inherent, block_time, receipt.as_ref(), store.write(txn))?;

        // The account might have been created by the inherent (i.e. reward inherent).
        self.put_or_prune(txn, address, account);

        Ok(())
    }

    pub fn finalize_batch(&self, txn: &mut WriteTransaction) {
        // It is fine to have an incomplete trie here.
        let _ = self.tree.update_root(txn);
    }

    pub fn commit_chunk(
        &self,
        txn: &mut WriteTransaction,
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
        txn: &mut WriteTransaction,
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
                    .get_chunk_with_proof(&ReadTransaction::new(&self.env), start_key.., limit)
            }
        }
    }
}
