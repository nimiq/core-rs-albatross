use nimiq_database::{
    Environment, ReadTransaction, Transaction as DBTransaction, WriteTransaction,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_transaction::{ExecutedTransaction, Transaction, TransactionFlags};
use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_trie::trie::MerkleRadixTrie;

use crate::{
    logs::{BatchInfo, TransactionLog},
    Account, AccountError, AccountInherentInteraction, AccountTransactionInteraction, Inherent,
    Log, Receipt, Receipts, RevertTransactionLogs, TransactionInfo,
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

    pub fn exercise_transactions(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<(Blake2bHash, Vec<ExecutedTransaction>), AccountError> {
        let mut txn = WriteTransaction::new(&self.env);

        let (_, executed_txns) =
            self.commit(&mut txn, transactions, inherents, block_height, timestamp)?;

        let hash = self.get_root(Some(&txn));

        txn.abort();

        Ok((hash, executed_txns))
    }

    /// This functions operates on a per transaction basis.
    /// It operates atomically i.e.: either the transaction is fully committed (sender / recipient) or
    /// it returns an error and no state is changed
    pub fn commit_transaction(
        &self,
        txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        timestamp: u64,
    ) -> Result<TransactionInfo, AccountError> {
        let mut transaction_info = TransactionInfo::new();

        // If either of these two fails, we return
        if transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            // If we have a contract creation we need to skip the recipients part
            transaction_info.create_info = Some(Account::create(
                &self.tree,
                txn,
                transaction,
                block_height,
                timestamp,
            )?);
        } else {
            // Commit recipient
            transaction_info.recipient_info = Some(Account::commit_incoming_transaction(
                &self.tree,
                txn,
                transaction,
                block_height,
                timestamp,
            )?);
        }

        // Commit sender
        // If this fails, we need to revert any change that was committed to the accounts tree
        match Account::commit_outgoing_transaction(
            &self.tree,
            txn,
            transaction,
            block_height,
            timestamp,
        ) {
            Ok(account_info) => {
                transaction_info.sender_info = Some(account_info);
            }
            Err(account_err) => {
                if transaction
                    .flags
                    .contains(TransactionFlags::CONTRACT_CREATION)
                {
                    self.tree
                        .remove(txn, &KeyNibbles::from(&transaction.recipient));
                } else {
                    Account::revert_incoming_transaction(
                        &self.tree,
                        txn,
                        transaction,
                        block_height,
                        timestamp,
                        transaction_info
                            .recipient_info
                            .expect("receipt is needed")
                            .receipt
                            .as_ref(),
                    )
                    .unwrap();
                }
                return Err(account_err);
            }
        };

        // Return data
        Ok(transaction_info)
    }

    pub fn commit(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<(BatchInfo, Vec<ExecutedTransaction>), AccountError> {
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
    ) -> Result<(BatchInfo, Vec<ExecutedTransaction>), AccountError> {
        // Fetches all pre inherents to commit.
        let pre_inherents: Vec<Inherent> = inherents
            .iter()
            .filter(|i| i.is_pre_transactions())
            .cloned()
            .collect();
        let mut batch_info = self.commit_inherents(txn, &pre_inherents, block_height, timestamp)?;

        let mut executed_txns = Vec::new();

        let mut sender_receipts = Vec::new();
        let mut sender_logs = Vec::new();

        let mut recipient_receipts = Vec::new();
        let mut recipient_logs = Vec::new();

        let mut create_logs = Vec::new();

        // Commit each transaction
        for (index, transaction) in transactions.iter().enumerate() {
            match self.commit_transaction(txn, transaction, block_height, timestamp) {
                Ok(txn_info) => {
                    // Collect txn receipt/logs
                    if let Some(sender_info) = txn_info.sender_info {
                        // Process sender logs first
                        sender_receipts.push(Receipt::Transaction {
                            index: index as u16,
                            sender: true,
                            data: sender_info.receipt,
                        });

                        sender_logs.push(TransactionLog::new(transaction.hash(), sender_info.logs));
                    }

                    // Process recipients logs (if any)
                    if let Some(recipient_info) = txn_info.recipient_info {
                        recipient_receipts.push(Receipt::Transaction {
                            index: index as u16,
                            sender: false,
                            data: recipient_info.receipt,
                        });

                        recipient_logs
                            .push(TransactionLog::new(transaction.hash(), recipient_info.logs));
                    }

                    // Process create logs (if any)
                    if let Some(info) = txn_info.create_info {
                        // Note that create contract transactions do not generate receipts
                        create_logs.push(TransactionLog::new(transaction.hash(), info.logs));
                    }

                    // Store the sucessful executed transaction
                    executed_txns.push(ExecutedTransaction::Ok(transaction.clone()));
                }
                Err(account_err) => {
                    // Commit the failed transaction (i.e deduce fee) and collect logs
                    match Account::commit_failed_transaction(
                        &self.tree,
                        txn,
                        transaction,
                        block_height,
                    ) {
                        Ok(account_info) => {
                            // Store the reason why the transaction failed
                            let mut logs = vec![Log::FailedTransaction {
                                from: transaction.sender.clone(),
                                to: transaction.recipient.clone(),
                                failure_reason: account_err.to_string(),
                            }];

                            logs.extend(account_info.logs);

                            // Store the executed failed transaction
                            executed_txns.push(ExecutedTransaction::Err(transaction.clone()));

                            // Collect receipts from applying a failed transaction (they go into the senders collection)
                            sender_receipts.push(Receipt::Transaction {
                                index: index as u16,
                                sender: true,
                                data: account_info.receipt,
                            });

                            recipient_receipts.push(Receipt::Transaction {
                                index: index as u16,
                                sender: false,
                                data: None,
                            });

                            sender_logs.push(TransactionLog::new(transaction.hash(), logs));
                        }
                        Err(err) => {
                            // This should not happen, as the mempool should verify that the sender can pay the fee
                            return Err(err);
                        }
                    }
                }
            };
        }

        let mut senders_info = BatchInfo::new(sender_receipts, sender_logs, Vec::new());
        batch_info.receipts.append(&mut senders_info.receipts);

        let mut recipients_info = BatchInfo::new(recipient_receipts, recipient_logs, Vec::new());
        batch_info.receipts.append(&mut recipients_info.receipts);

        let mut create_info = BatchInfo::new(Vec::new(), create_logs, Vec::new());
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
        Ok((
            Self::check_merge_tx_logs(
                batch_info,
                senders_info.tx_logs,
                recipients_info.tx_logs,
                create_info.tx_logs,
            ),
            executed_txns,
        ))
    }

    pub fn revert_transaction(
        &self,
        db_txn: &mut WriteTransaction,
        transaction: &ExecutedTransaction,
        block_height: u32,
        timestamp: u64,
        receipts: (Receipt, Option<Receipt>),
    ) -> RevertTransactionLogs {
        let (sender_receipt, recipient_receipt) = receipts;

        let mut logs = RevertTransactionLogs::new();

        match transaction {
            ExecutedTransaction::Ok(txn) => {
                if let Some(recipient_receipt) = recipient_receipt {
                    // Revert recipients
                    match recipient_receipt {
                        Receipt::Transaction {
                            index: _,
                            sender: _,
                            data,
                        } => {
                            logs.recipient_log = Account::revert_incoming_transaction(
                                &self.tree,
                                db_txn,
                                txn,
                                block_height,
                                timestamp,
                                data.as_ref(),
                            )
                            .unwrap_or_else(|_| {
                                log::error!(
                                    " Failed to revert a succesfull incoming transaction: {:?}",
                                    txn
                                );
                                panic!();
                            })
                        }

                        _ => {
                            unreachable!()
                        }
                    }
                }

                match sender_receipt {
                    Receipt::Transaction {
                        index: _,
                        sender: _,
                        data,
                    } => {
                        // Revert senders
                        logs.sender_log = Account::revert_outgoing_transaction(
                            &self.tree,
                            db_txn,
                            txn,
                            block_height,
                            timestamp,
                            data.as_ref(),
                        )
                        .unwrap_or_else(|_| {
                            log::error!(
                                " Failed to revert a succesfull outgoing transaction: {:?}",
                                txn
                            );
                            panic!();
                        })
                    }
                    _ => {
                        unreachable!()
                    }
                }
            }
            ExecutedTransaction::Err(txn) => {
                match sender_receipt {
                    Receipt::Transaction {
                        index: _,
                        sender: _,
                        data,
                    } => {
                        // Revert failed transaction, logs are stored inside the senders container
                        logs.sender_log = Account::revert_failed_transaction(
                            &self.tree,
                            db_txn,
                            txn,
                            data.as_ref(),
                        )
                        .unwrap_or_else(|_| {
                            log::error!(" Failed to revert a failed transaction: {:?}", txn);
                            panic!();
                        })
                    }
                    _ => {
                        unreachable!()
                    }
                }
            }
        }
        logs
    }

    pub fn revert(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[ExecutedTransaction],
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
        transactions: &[ExecutedTransaction],
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
        let mut recipients_logs = Vec::new();
        let mut senders_logs = Vec::new();
        let mut contracts_logs = Vec::new();

        let mut recipient_index = 0_usize;
        let mut recipient_receipt;

        // Safety check, we should always have one sender receipt per transaction
        if transactions.len() != sender_receipts.len() {
            panic!(" The number of sender receipts does not match the number of transactions");
        }

        //Iterate the transactions in the reverse order they were applied
        for (index, transaction) in transactions.iter().rev().enumerate() {
            // Sender receipts are always present, however a recipient receipt might or might not be present:
            // it is not present when the transaction is a contract creation transaction, so we need to track
            // recipient receipts using a different index,
            let sender_receipt = sender_receipts[index].clone();

            match sender_receipt {
                Receipt::Transaction {
                    index: receipt_index,
                    sender: _,
                    data: _,
                } => {
                    assert_eq!((transactions.len() - 1) - index, receipt_index as usize)
                }
                _ => panic!(" Only transaction receipts should be present "),
            }

            if transaction
                .get_raw_transaction()
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                match transaction {
                    ExecutedTransaction::Ok(transaction) => {
                        Account::delete(&self.tree, txn, transaction).unwrap();

                        contracts_logs.push(TransactionLog::new(
                            transaction.hash(),
                            vec![Log::RevertContract {
                                contract_address: transaction.recipient.clone(),
                            }],
                        ));
                    }
                    ExecutedTransaction::Err(_) => {
                        // If the contract creation failed, we have nothing to do
                    }
                }

                recipient_receipt = None;
            } else {
                let receipt = recipient_receipts[recipient_index].clone();
                match receipt {
                    Receipt::Transaction {
                        index: receipt_index,
                        sender: _,
                        data: _,
                    } => {
                        assert_eq!((transactions.len() - 1) - index, receipt_index as usize)
                    }
                    _ => panic!(" Only transaction receipts should be present "),
                }
                recipient_receipt = Some(receipt);
                recipient_index += 1;
            }

            let logs = self.revert_transaction(
                txn,
                transaction,
                block_height,
                timestamp,
                (sender_receipt, recipient_receipt),
            );

            if !logs.recipient_log.is_empty() {
                recipients_logs.push(TransactionLog::new(
                    transaction.get_raw_transaction().hash(),
                    logs.recipient_log,
                ));
            }

            senders_logs.push(TransactionLog::new(
                transaction.get_raw_transaction().hash(),
                logs.sender_log,
            ));
        }

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
            senders_logs,
            recipients_logs,
            contracts_logs,
        ))
    }

    pub fn finalize_batch(&self, txn: &mut WriteTransaction) {
        self.tree.update_root(txn);
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

    /// Merges the log transactions from senders, receiptients and create_contracts and returns batch_info containing the merged tx_logs.
    /// This means that the batch_info inherent_logs and receipts are preserved and returned by this method.
    /// The batch_info provided as argument is expected to have its tx_logs vec empty, otherwise its content will get overwritten.
    fn check_merge_tx_logs(
        mut batch_info: BatchInfo,
        mut senders_logs: Vec<TransactionLog>,
        mut recipients_logs: Vec<TransactionLog>,
        mut create_logs: Vec<TransactionLog>,
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
        // Therefore, all tx_logs of these two vecs are now expected to be processed and appended senders_logs tx_logs.
        assert_eq!(None, tx_logs_recipients.peek(), "recipients tx_logs were not fully processed. Possible cause is a ordering difference compared to senders_logs tx_logs");
        assert_eq!(None, tx_logs_create.peek(), "create tx_logs were not fully processed. Possible cause is a ordering difference compared to senders_logs tx_logs");

        batch_info.tx_logs = senders_logs.to_vec();
        batch_info
    }
}
