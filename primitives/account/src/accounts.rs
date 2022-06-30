use nimiq_database::{
    Environment, ReadTransaction, Transaction as DBTransaction, WriteTransaction,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_transaction::{Transaction, TransactionFlags};
use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_trie::trie::MerkleRadixTrie;

use crate::{
    logs::{BatchInfo, TransactionLog},
    Account, AccountError, AccountInherentInteraction, AccountTransactionInteraction, Inherent,
    Log, Receipt, Receipts,
};

/// An alias for the accounts tree.
pub type AccountsTrie = MerkleRadixTrie<Account>;

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
    /// Creates a new, completely empty Accounts.
    pub fn new(env: Environment) -> Self {
        let tree = AccountsTrie::new(env.clone(), "AccountsTrie");
        Accounts { env, tree }
    }

    /// Initializes the Accounts struct with a given list of accounts.
    pub fn init(&self, txn: &mut WriteTransaction, genesis_accounts: Vec<(KeyNibbles, Account)>) {
        log::debug!("Initializing Accounts");
        for (key, account) in genesis_accounts {
            log::trace!(
                "Adding new account: KeyNibbles: {:?}, Account: {:?}",
                &key,
                &account
            );
            self.tree.put(txn, &key, account);
        }
        self.tree.update_root(txn);
    }

    /// Returns the number of accounts in the Accounts Trie. It will traverse the entire tree.
    pub fn size(&self, txn_option: Option<&DBTransaction>) -> usize {
        match txn_option {
            Some(txn) => self.tree.size(txn),
            None => self.tree.size(&ReadTransaction::new(&self.env)),
        }
    }

    pub fn get(&self, key: &KeyNibbles, txn_option: Option<&DBTransaction>) -> Option<Account> {
        match txn_option {
            Some(txn) => self.tree.get(txn, key),
            None => self.tree.get(&ReadTransaction::new(&self.env), key),
        }
    }

    pub fn get_root(&self, txn_option: Option<&DBTransaction>) -> Blake2bHash {
        match txn_option {
            Some(txn) => self.tree.root_hash(txn),
            None => self.tree.root_hash(&ReadTransaction::new(&self.env)),
        }
    }

    pub fn get_root_with(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<Blake2bHash, AccountError> {
        let mut txn = WriteTransaction::new(&self.env);

        self.commit(&mut txn, transactions, inherents, block_height, timestamp)?;

        let hash = self.get_root(Some(&txn));

        txn.abort();

        Ok(hash)
    }

    pub fn commit(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<BatchInfo, AccountError> {
        let result = self.commit_batch(txn, transactions, inherents, block_height, timestamp);
        self.tree.update_root(txn);
        result
    }

    pub fn commit_batch(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<BatchInfo, AccountError> {
        // Fetches all pre inherents to commit.
        let pre_inherents: Vec<Inherent> = inherents
            .iter()
            .filter(|i| i.is_pre_transactions())
            .cloned()
            .collect();
        let mut batch_info = self.commit_inherents(txn, &pre_inherents, block_height, timestamp)?;

        // Commits all senders, recipients and create contracts and appends all receipts to batch_info.
        let mut senders_info = self.commit_senders(txn, transactions, block_height, timestamp)?;
        batch_info.receipts.append(&mut senders_info.receipts);

        let mut recipients_info =
            self.commit_recipients(txn, transactions, block_height, timestamp)?;
        batch_info.receipts.append(&mut recipients_info.receipts);

        let mut create_info = self.create_contracts(txn, transactions, block_height, timestamp)?;
        batch_info.receipts.append(&mut create_info.receipts);

        // Fetches all pos inherents to commit.
        let post_inherents: Vec<Inherent> = inherents
            .iter()
            .filter(|i| !i.is_pre_transactions())
            .cloned()
            .collect();
        let mut post_inherents_info =
            self.commit_inherents(txn, &post_inherents, block_height, timestamp)?;

        // Appends newly generated inherents logs and receipts.
        batch_info
            .receipts
            .append(&mut post_inherents_info.receipts);

        batch_info
            .inherent_logs
            .append(&mut post_inherents_info.inherent_logs);

        // Merges all tx_logs into batch_info.
        Ok(Self::check_merge_tx_logs(
            batch_info,
            &mut senders_info.tx_logs,
            &mut recipients_info.tx_logs,
            &mut create_info.tx_logs,
        ))
    }

    pub fn revert(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
        receipts: &Receipts,
    ) -> Result<BatchInfo, AccountError> {
        let logs = self.revert_batch(
            txn,
            transactions,
            inherents,
            block_height,
            timestamp,
            receipts,
        )?;
        self.tree.update_root(txn);
        Ok(logs)
    }

    pub fn revert_batch(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
        receipts: &Receipts,
    ) -> Result<BatchInfo, AccountError> {
        // Organizes the receipts by the different types of inherents (pre or post) and transactions (senders or recipients) receipts.
        let (
            sender_receipts,
            recipient_receipts,
            pre_tx_inherent_receipts,
            post_tx_inherent_receipts,
        ) = Self::prepare_receipts(receipts);

        // Reverts transactions in the inverse order of the commit.
        let logs_inherent = self.revert_inherents(
            txn,
            inherents,
            block_height,
            timestamp,
            post_tx_inherent_receipts,
        )?;

        // Complete batch_info for the revert. This batch_info has the receipts always empty, since there is nothing else to revert.
        let mut batch_info = BatchInfo::new(Vec::new(), Vec::new(), logs_inherent);

        let mut contracts_logs =
            self.revert_contracts(txn, transactions, block_height, timestamp)?;

        let mut recipients_logs = self.revert_recipients(
            txn,
            transactions,
            block_height,
            timestamp,
            recipient_receipts,
        )?;

        let mut senders_logs =
            self.revert_senders(txn, transactions, block_height, timestamp, sender_receipts)?;

        // Gathers all the inherent_logs into the batch_info return object.
        batch_info.inherent_logs.append(&mut self.revert_inherents(
            txn,
            inherents,
            block_height,
            timestamp,
            pre_tx_inherent_receipts,
        )?);

        // Merges all tx_logs into batch_info.
        Ok(Self::check_merge_tx_logs(
            batch_info,
            &mut senders_logs,
            &mut recipients_logs,
            &mut contracts_logs,
        ))
    }

    pub fn finalize_batch(&self, txn: &mut WriteTransaction) {
        self.tree.update_root(txn);
    }

    fn commit_senders(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
    ) -> Result<BatchInfo, AccountError> {
        let mut receipts = Vec::new();
        let mut logs = Vec::new();

        for (index, transaction) in transactions.iter().enumerate() {
            let data = Account::commit_outgoing_transaction(
                &self.tree,
                txn,
                transaction,
                block_height,
                timestamp,
            )?;

            receipts.push(Receipt::Transaction {
                index: index as u16,
                sender: true,
                data: data.receipt,
            });

            logs.push(TransactionLog::new(transaction.hash(), data.logs));
        }

        Ok(BatchInfo::new(receipts, logs, Vec::new()))
    }

    fn revert_senders(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
        receipts: Vec<Receipt>,
    ) -> Result<Vec<TransactionLog>, AccountError> {
        let mut tx_logs = Vec::new();
        for receipt in receipts {
            match receipt {
                Receipt::Transaction {
                    index,
                    sender,
                    data,
                } => {
                    assert!(sender);

                    let logs = Account::revert_outgoing_transaction(
                        &self.tree,
                        txn,
                        &transactions[index as usize],
                        block_height,
                        timestamp,
                        data.as_ref(),
                    )?;
                    tx_logs.push(TransactionLog::new(
                        transactions[index as usize].hash(),
                        logs,
                    ));
                }
                _ => {
                    unreachable!()
                }
            };
        }

        Ok(tx_logs)
    }

    fn commit_recipients(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
    ) -> Result<BatchInfo, AccountError> {
        let mut receipts = Vec::new();
        let mut logs = Vec::new();

        for (index, transaction) in transactions.iter().enumerate() {
            // If this is a contract creation transaction, skip it.
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                continue;
            }

            let account_info = Account::commit_incoming_transaction(
                &self.tree,
                txn,
                transaction,
                block_height,
                timestamp,
            )?;

            receipts.push(Receipt::Transaction {
                index: index as u16,
                sender: false,
                data: account_info.receipt,
            });

            logs.push(TransactionLog::new(transaction.hash(), account_info.logs));
        }

        Ok(BatchInfo::new(receipts, logs, Vec::new()))
    }

    fn revert_recipients(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
        receipts: Vec<Receipt>,
    ) -> Result<Vec<TransactionLog>, AccountError> {
        let mut tx_logs = Vec::new();
        for receipt in receipts {
            match receipt {
                Receipt::Transaction {
                    index,
                    sender,
                    data,
                } => {
                    assert!(!sender);

                    let logs = Account::revert_incoming_transaction(
                        &self.tree,
                        txn,
                        &transactions[index as usize],
                        block_height,
                        timestamp,
                        data.as_ref(),
                    )?;
                    tx_logs.push(TransactionLog::new(
                        transactions[index as usize].hash(),
                        logs,
                    ));
                }
                _ => {
                    unreachable!()
                }
            };
        }

        Ok(tx_logs)
    }

    fn create_contracts(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
    ) -> Result<BatchInfo, AccountError> {
        let mut tx_logs = Vec::new();

        for transaction in transactions {
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                let account_info =
                    Account::create(&self.tree, txn, transaction, block_height, timestamp)?;
                tx_logs.push(TransactionLog::new(transaction.hash(), account_info.logs));
            }
        }
        Ok(BatchInfo::new(Vec::new(), tx_logs, Vec::new()))
    }

    fn revert_contracts(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        _block_height: u32,
        _timestamp: u64,
    ) -> Result<Vec<TransactionLog>, AccountError> {
        let mut tx_logs = Vec::new();
        for transaction in transactions.iter().rev() {
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                self.tree
                    .remove(txn, &KeyNibbles::from(&transaction.recipient));
                tx_logs.push(TransactionLog::new(
                    transaction.hash(),
                    vec![Log::RevertContract {
                        contract_address: transaction.recipient.clone(),
                    }],
                ));
            }
        }

        Ok(tx_logs)
    }

    fn commit_inherents(
        &self,
        txn: &mut WriteTransaction,
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<BatchInfo, AccountError> {
        let mut receipts = Vec::new();
        let mut logs = Vec::new();

        for (index, inherent) in inherents.iter().enumerate() {
            let mut account_info =
                Account::commit_inherent(&self.tree, txn, inherent, block_height, timestamp)?;

            receipts.push(Receipt::Inherent {
                index: index as u16,
                pre_transactions: inherent.is_pre_transactions(),
                data: account_info.receipt,
            });

            logs.append(&mut account_info.logs);
        }

        Ok(BatchInfo::new(receipts, Vec::new(), logs))
    }

    fn revert_inherents(
        &self,
        txn: &mut WriteTransaction,
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
        receipts: Vec<Receipt>,
    ) -> Result<Vec<Log>, AccountError> {
        let mut inherent_logs = Vec::new();

        for receipt in receipts {
            inherent_logs.append(&mut match receipt {
                Receipt::Inherent { index, data, .. } => Account::revert_inherent(
                    &self.tree,
                    txn,
                    &inherents[index as usize],
                    block_height,
                    timestamp,
                    data.as_ref(),
                )?,
                _ => {
                    unreachable!()
                }
            });
        }

        Ok(inherent_logs)
    }

    fn prepare_receipts(
        receipts: &Receipts,
    ) -> (Vec<Receipt>, Vec<Receipt>, Vec<Receipt>, Vec<Receipt>) {
        let mut sender_receipts = Vec::new();
        let mut recipient_receipts = Vec::new();
        let mut pre_tx_inherent_receipts = Vec::new();
        let mut post_tx_inherent_receipts = Vec::new();

        for receipt in &receipts.receipts {
            match receipt {
                Receipt::Transaction { sender, .. } => {
                    if *sender {
                        sender_receipts.push(receipt.clone());
                    } else {
                        recipient_receipts.push(receipt.clone());
                    }
                }
                Receipt::Inherent {
                    pre_transactions, ..
                } => {
                    if *pre_transactions {
                        pre_tx_inherent_receipts.push(receipt.clone());
                    } else {
                        post_tx_inherent_receipts.push(receipt.clone());
                    }
                }
            }
        }

        // We put the vector in reverse order so that it reverse matches the order in which they
        // were applied to the block. The performance of sorting then reversing is actually quite
        // good.
        sender_receipts.sort();
        sender_receipts.reverse();
        recipient_receipts.sort();
        recipient_receipts.reverse();
        pre_tx_inherent_receipts.sort();
        pre_tx_inherent_receipts.reverse();
        post_tx_inherent_receipts.sort();
        post_tx_inherent_receipts.reverse();

        (
            sender_receipts,
            recipient_receipts,
            pre_tx_inherent_receipts,
            post_tx_inherent_receipts,
        )
    }

    /// Merge the log transactions from senders, receiptients and create_contracts and returns logs organized by Transaction logs.
    /// The final result is moved to the senders_logs and returned, leaving the other vectors empty.
    fn check_merge_tx_logs(
        mut batch_info: BatchInfo,
        senders_logs: &mut [TransactionLog],
        recipients_logs: &mut [TransactionLog],
        create_logs: &mut [TransactionLog],
    ) -> BatchInfo {
        // The senders has all transaction log entries. We iterate over it and append the remaing logs to the respective tx_log of sender_logs.
        let mut tx_logs_recipients = recipients_logs.iter_mut().peekable();
        let mut tx_logs_create = create_logs.iter_mut().peekable();

        for tx_log_sender in senders_logs.iter_mut() {
            if let Some(tx_log) = tx_logs_recipients.peek_mut() {
                if tx_log.tx_hash == tx_log_sender.tx_hash {
                    tx_log_sender.logs.append(&mut tx_log.logs);
                    tx_logs_recipients.next();
                    // The transactions are wither a create or anything else (mutually exclusive), thus we can avoid comparing to the tx_create.
                    continue;
                }
            }

            if let Some(tx_log) = tx_logs_create.peek_mut() {
                if tx_log.tx_hash == tx_log_sender.tx_hash {
                    tx_log_sender.logs.append(&mut tx_log.logs);
                    tx_logs_create.next();
                }
            }
        }

        // All receipient and create tx_logs should have a matching tx_log from the senders.
        // Therefore, all tx_logs of these two vecs are now expected to be appended and moved to the senders_logs.
        assert_eq!(None, tx_logs_recipients.peek());
        assert_eq!(None, tx_logs_create.peek());

        batch_info.tx_logs = senders_logs.to_vec();
        batch_info
    }
}
