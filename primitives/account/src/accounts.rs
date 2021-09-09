use nimiq_database::{
    Environment, ReadTransaction, Transaction as DBTransaction, WriteTransaction,
};
use nimiq_hash::Blake2bHash;
use nimiq_transaction::{Transaction, TransactionFlags};
use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_trie::trie::MerkleRadixTrie;

use crate::{
    Account, AccountError, AccountInherentInteraction, AccountTransactionInteraction, Inherent,
    Receipt, Receipts,
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
        for (key, account) in genesis_accounts {
            self.tree.put(txn, &key, account);
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
    ) -> Result<Receipts, AccountError> {
        let mut receipts = Vec::new();

        let pre_inherents: Vec<Inherent> = inherents
            .iter()
            .filter(|i| i.is_pre_transactions())
            .cloned()
            .collect();

        receipts.append(&mut self.commit_inherents(
            txn,
            &pre_inherents,
            block_height,
            timestamp,
        )?);

        receipts.append(&mut self.commit_senders(txn, transactions, block_height, timestamp)?);

        receipts.append(&mut self.commit_recipients(txn, transactions, block_height, timestamp)?);

        self.create_contracts(txn, transactions, block_height, timestamp)?;

        let post_inherents: Vec<Inherent> = inherents
            .iter()
            .filter(|i| !i.is_pre_transactions())
            .cloned()
            .collect();

        receipts.append(&mut self.commit_inherents(
            txn,
            &post_inherents,
            block_height,
            timestamp,
        )?);

        Ok(Receipts::from(receipts))
    }

    pub fn revert(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
        receipts: &Receipts,
    ) -> Result<(), AccountError> {
        let (
            sender_receipts,
            recipient_receipts,
            pre_tx_inherent_receipts,
            post_tx_inherent_receipts,
        ) = Self::prepare_receipts(receipts);

        self.revert_inherents(
            txn,
            inherents,
            block_height,
            timestamp,
            post_tx_inherent_receipts,
        )?;

        self.revert_contracts(txn, transactions, block_height, timestamp)?;

        self.revert_recipients(
            txn,
            transactions,
            block_height,
            timestamp,
            recipient_receipts,
        )?;

        self.revert_senders(txn, transactions, block_height, timestamp, sender_receipts)?;

        self.revert_inherents(
            txn,
            inherents,
            block_height,
            timestamp,
            pre_tx_inherent_receipts,
        )?;

        Ok(())
    }

    fn commit_senders(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
    ) -> Result<Vec<Receipt>, AccountError> {
        let mut receipts = Vec::new();

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
                data,
            });
        }

        Ok(receipts)
    }

    fn revert_senders(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
        receipts: Vec<Receipt>,
    ) -> Result<(), AccountError> {
        for receipt in receipts {
            match receipt {
                Receipt::Transaction {
                    index,
                    sender,
                    data,
                } => {
                    assert!(sender);

                    Account::revert_outgoing_transaction(
                        &self.tree,
                        txn,
                        &transactions[index as usize],
                        block_height,
                        timestamp,
                        data.as_ref(),
                    )?
                }
                _ => {
                    unreachable!()
                }
            }
        }

        Ok(())
    }

    fn commit_recipients(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
    ) -> Result<Vec<Receipt>, AccountError> {
        let mut receipts = Vec::new();

        for (index, transaction) in transactions.iter().enumerate() {
            // If this is a contract creation transaction, skip it.
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                continue;
            }

            let data = Account::commit_incoming_transaction(
                &self.tree,
                txn,
                transaction,
                block_height,
                timestamp,
            )?;

            receipts.push(Receipt::Transaction {
                index: index as u16,
                sender: false,
                data,
            });
        }

        Ok(receipts)
    }

    fn revert_recipients(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
        receipts: Vec<Receipt>,
    ) -> Result<(), AccountError> {
        for receipt in receipts {
            match receipt {
                Receipt::Transaction {
                    index,
                    sender,
                    data,
                } => {
                    assert!(!sender);

                    Account::revert_incoming_transaction(
                        &self.tree,
                        txn,
                        &transactions[index as usize],
                        block_height,
                        timestamp,
                        data.as_ref(),
                    )?
                }
                _ => {
                    unreachable!()
                }
            }
        }

        Ok(())
    }

    fn create_contracts(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        block_height: u32,
        timestamp: u64,
    ) -> Result<(), AccountError> {
        for transaction in transactions {
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                Account::create(&self.tree, txn, transaction, block_height, timestamp)?
            }
        }
        Ok(())
    }

    fn revert_contracts(
        &self,
        txn: &mut WriteTransaction,
        transactions: &[Transaction],
        _block_height: u32,
        _timestamp: u64,
    ) -> Result<(), AccountError> {
        for transaction in transactions.iter().rev() {
            if transaction
                .flags
                .contains(TransactionFlags::CONTRACT_CREATION)
            {
                self.tree
                    .remove(txn, &KeyNibbles::from(&transaction.recipient));
            }
        }

        Ok(())
    }

    fn commit_inherents(
        &self,
        txn: &mut WriteTransaction,
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
    ) -> Result<Vec<Receipt>, AccountError> {
        let mut receipts = Vec::new();

        for (index, inherent) in inherents.iter().enumerate() {
            let data =
                Account::commit_inherent(&self.tree, txn, inherent, block_height, timestamp)?;

            receipts.push(Receipt::Inherent {
                index: index as u16,
                pre_transactions: inherent.is_pre_transactions(),
                data,
            });
        }

        Ok(receipts)
    }

    fn revert_inherents(
        &self,
        txn: &mut WriteTransaction,
        inherents: &[Inherent],
        block_height: u32,
        timestamp: u64,
        receipts: Vec<Receipt>,
    ) -> Result<(), AccountError> {
        for receipt in receipts {
            match receipt {
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
            }
        }

        Ok(())
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
}
