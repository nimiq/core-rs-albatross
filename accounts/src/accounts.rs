use std::collections::HashMap;
use std::sync::Arc;

use account::{Account, AccountError, AccountTransactionInteraction, AccountType, PrunedAccount, Receipt, Receipts};
use account::inherent::{AccountInherentInteraction, Inherent};
use database::{Environment, ReadTransaction, WriteTransaction};
use database as db;
use hash::Blake2bHash;
use keys::Address;
use transaction::{Transaction, TransactionFlags};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;

use crate::tree::AccountsTree;


type ReceiptsMap<'a> = HashMap<u16, &'a Vec<u8>>;

#[derive(Debug)]
pub struct Accounts {
    env: Environment,
    tree: AccountsTree,
}

impl Accounts {
    pub fn new(env: Environment) -> Self {
        let tree = AccountsTree::new(env.clone());
        Accounts { env, tree, }
    }

    pub fn init(&self, txn: &mut WriteTransaction, genesis_accounts: Vec<(Address, Account)>) {
        for (address, account) in genesis_accounts {
            self.tree.put_batch(txn, &address, account);
        }
        self.tree.finalize_batch(txn);
    }

    pub fn get(&self, address: &Address, txn_option: Option<&db::Transaction>) -> Account {
        match txn_option {
            Some(txn) => self.tree.get(txn, address),
            None => self.tree.get(&ReadTransaction::new(&self.env), address)
        }.unwrap_or(Account::INITIAL)
    }

    pub fn get_chunk(&self, prefix: &str, size: usize, txn_option: Option<&db::Transaction>) -> Option<AccountsTreeChunk> {
        match txn_option {
            Some(txn) => self.tree.get_chunk(txn, prefix, size),
            None => self.tree.get_chunk(&ReadTransaction::new(&self.env), prefix, size),
        }
    }

    pub fn get_accounts_proof(&self, txn: &db::Transaction, addresses: &[Address]) -> AccountsProof {
        self.tree.get_accounts_proof(txn, addresses)
    }

    pub fn hash(&self, txn_option: Option<&db::Transaction>) -> Blake2bHash {
        match txn_option {
            Some(txn) => self.tree.root_hash(txn),
            None => self.tree.root_hash(&ReadTransaction::new(&self.env))
        }
    }

    pub fn hash_with(&self, transactions: &[Transaction], inherents: &[Inherent], block_height: u32) -> Result<Blake2bHash, AccountError> {
        let mut txn = WriteTransaction::new(&self.env);
        self.commit(&mut txn, transactions, inherents, block_height)?;
        let hash = self.hash(Some(&txn));
        txn.abort();
        Ok(hash)
    }

    /// Returns receipts in processing order, *not* block order!
    pub fn collect_receipts(&self, transactions: &[Transaction], inherents: &[Inherent], block_height: u32) -> Result<Receipts, AccountError> {
        let mut txn = WriteTransaction::new(&self.env);
        let receipts = self.commit_nonfinal(&mut txn, transactions, inherents, block_height)?;
        txn.abort();
        Ok(receipts)
    }

    pub fn commit(&self, txn: &mut WriteTransaction, transactions: &[Transaction], inherents: &[Inherent], block_height: u32) -> Result<Receipts, AccountError> {
        let receipts = self.commit_nonfinal(txn, transactions, inherents, block_height)?;
        self.tree.finalize_batch(txn);
        Ok(receipts)
    }

    pub fn revert(&self, txn: &mut WriteTransaction, transactions: &[Transaction], inherents: &[Inherent], block_height: u32, receipts: &Receipts) -> Result<(), AccountError> {
        self.revert_nonfinal(txn, transactions, inherents, block_height, receipts)?;
        self.tree.finalize_batch(txn);
        Ok(())
    }

    fn commit_nonfinal(&self, txn: &mut WriteTransaction, transactions: &[Transaction], inherents: &[Inherent], block_height: u32) -> Result<Receipts, AccountError> {
        let mut receipts = Vec::new();

        receipts.append(&mut self.process_senders(txn, transactions, block_height, HashMap::new(),
                                                  |account, transaction, block_height, _| account.commit_outgoing_transaction(transaction, block_height))?);

        receipts.append(&mut self.process_recipients(txn, transactions, block_height, HashMap::new(),
                                                     |account, transaction, block_height, _| account.commit_incoming_transaction(transaction, block_height))?);

        self.create_contracts(txn, transactions, block_height)?;

        // TODO It makes more sense to prune accounts *after* inherents have been processed.
        // However, v1 awards the block reward after pruning, so we keep this behavior for now.
        receipts.append(&mut self.prune_accounts(txn, transactions)?);

        receipts.append(&mut self.process_inherents(txn, inherents, HashMap::new(),
                                                    |account, inherent, _| account.commit_inherent(inherent))?);

        Ok(Receipts::from(receipts))
    }

    fn revert_nonfinal(&self, txn: &mut WriteTransaction, transactions: &[Transaction], inherents: &[Inherent], block_height: u32, receipts: &Receipts) -> Result<(), AccountError> {
        let (sender_receipts, recipient_receipts, inherent_receipts, pruned_accounts) = Self::prepare_receipts(receipts);

        self.process_inherents(txn, inherents, inherent_receipts,
                               |account, inherent, receipt| account.revert_inherent(inherent, receipt).map(|_| None))?;

        self.restore_accounts(txn, pruned_accounts)?;

        self.revert_contracts(txn, transactions, block_height)?;

        self.process_recipients(txn, transactions, block_height, recipient_receipts,
                                |account, transaction, block_height, receipt| account.revert_incoming_transaction(transaction, block_height, receipt).map(|_| None))?;

        self.process_senders(txn, transactions, block_height, sender_receipts,
                             |account, transaction, block_height, receipt| account.revert_outgoing_transaction(transaction, block_height, receipt).map(|_| None))?;

        Ok(())
    }

    fn process_senders<F>(&self, txn: &mut WriteTransaction, transactions: &[Transaction], block_height: u32, mut receipts: HashMap<u16, &Vec<u8>>, account_op: F) -> Result<Vec<Receipt>, AccountError>
        where F: Fn(&mut Account, &Transaction, u32, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        let mut new_receipts = Vec::new();
        for (index, transaction) in transactions.iter().enumerate() {
            if let Some(data) = self.process_transaction(txn, &transaction.sender, Some(transaction.sender_type), transaction, block_height, receipts.remove(&(index as u16)), &account_op)? {
                new_receipts.push(Receipt::Transaction {
                    index: index as u16,
                    sender: true,
                    data,
                });
            }
        }
        Ok(new_receipts)
    }

    fn process_recipients<F>(&self, txn: &mut WriteTransaction, transactions: &[Transaction], block_height: u32, mut receipts: HashMap<u16, &Vec<u8>>, account_op: F) -> Result<Vec<Receipt>, AccountError>
        where F: Fn(&mut Account, &Transaction, u32, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        let mut new_receipts = Vec::new();
        for (index, transaction) in transactions.iter().enumerate() {
            // FIXME This doesn't check that account_type == transaction.recipient_type when reverting
            let recipient_type = if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                None
            } else {
                Some(transaction.recipient_type)
            };

            if let Some(data) = self.process_transaction(txn, &transaction.recipient, recipient_type, transaction, block_height, receipts.remove(&(index as u16)), &account_op)? {
                new_receipts.push(Receipt::Transaction {
                    index: index as u16,
                    sender: false,
                    data,
                });
            }
        }
        Ok(new_receipts)
    }

    fn process_transaction<F>(&self, txn: &mut WriteTransaction, address: &Address, account_type: Option<AccountType>, transaction: &Transaction, block_height: u32, receipt: Option<&Vec<u8>>, account_op: &F) -> Result<Option<Vec<u8>>, AccountError>
        where F: Fn(&mut Account, &Transaction, u32, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        // TODO Eliminate copy
        let mut account = self.get(address, Some(txn));

        // Check account type.
        if let Some(account_type) = account_type {
            if account.account_type() != account_type {
                return Err(AccountError::TypeMismatch {expected: account.account_type(), got: account_type});
            }
        }

        // Apply transaction.
        let receipt = account_op(&mut account, transaction, block_height, receipt)?;

        // TODO Eliminate copy
        self.tree.put_batch(txn, address, account);

        Ok(receipt)
    }

    fn create_contracts(&self, txn: &mut WriteTransaction, transactions: &[Transaction], block_height: u32) -> Result<(), AccountError> {
        for transaction in transactions {
            if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                self.create_contract(txn, transaction, block_height)?;
            }
        }
        Ok(())
    }

    fn create_contract(&self, txn: &mut WriteTransaction, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        assert!(transaction.flags.contains(TransactionFlags::CONTRACT_CREATION));

        let recipient_account = self.get(&transaction.recipient, Some(txn));
        let new_recipient_account = Account::new_contract(transaction.recipient_type, recipient_account.balance(), transaction, block_height)?;
        self.tree.put_batch(txn, &transaction.recipient, new_recipient_account);
        Ok(())
    }

    fn revert_contracts(&self, txn: &mut WriteTransaction, transactions: &[Transaction], block_height: u32) -> Result<(), AccountError> {
        for transaction in transactions {
            if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                self.revert_contract(txn, transaction, block_height)?;
            }
        }
        Ok(())
    }

    fn revert_contract(&self, txn: &mut WriteTransaction, transaction: &Transaction, _block_height: u32) -> Result<(), AccountError> {
        assert!(transaction.flags.contains(TransactionFlags::CONTRACT_CREATION));

        let recipient_account = self.get(&transaction.recipient, Some(txn));
        if recipient_account.account_type() != transaction.recipient_type {
            return Err(AccountError::TypeMismatch { expected: recipient_account.account_type(), got: transaction.recipient_type });
        }

        let new_recipient_account = Account::new_basic(recipient_account.balance());
        self.tree.put_batch(txn, &transaction.recipient, new_recipient_account);
        Ok(())
    }

    fn prune_accounts(&self, txn: &mut WriteTransaction, transactions: &[Transaction]) -> Result<Vec<Receipt>, AccountError> {
        let mut receipts = Vec::new();
        for transaction in transactions {
            let sender_account = self.get(&transaction.sender, Some(txn));
            if sender_account.is_to_be_pruned() {
                // Produce receipt.
                receipts.push(Receipt::PrunedAccount(PrunedAccount {
                    address: transaction.sender.clone(),
                    account: sender_account
                }));

                // Prune account.
                self.tree.put_batch(txn, &transaction.sender, Account::INITIAL);
            }
        }
        Ok(receipts)
    }

    fn restore_accounts(&self, txn: &mut WriteTransaction, pruned_accounts: Vec<&PrunedAccount>) -> Result<(), AccountError> {
        for pruned_account in pruned_accounts {
            self.tree.put_batch(txn, &pruned_account.address, pruned_account.account.clone());
        }
        Ok(())
    }

    fn process_inherents<F>(&self, txn: &mut WriteTransaction, inherents: &[Inherent], mut receipts: HashMap<u16, &Vec<u8>>, account_op: F) -> Result<Vec<Receipt>, AccountError>
        where F: Fn(&mut Account, &Inherent, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        let mut new_receipts = Vec::new();
        for (index, inherent) in inherents.iter().enumerate() {
            if let Some(data) = self.process_inherent(txn, inherent, receipts.remove(&(index as u16)), &account_op)? {
                new_receipts.push(Receipt::Inherent {
                    index: index as u16,
                    data,
                });
            }
        }
        Ok(new_receipts)
    }

    fn process_inherent<F>(&self, txn: &mut WriteTransaction, inherent: &Inherent, receipt: Option<&Vec<u8>>, account_op: &F) -> Result<Option<Vec<u8>>, AccountError>
        where F: Fn(&mut Account, &Inherent, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        // TODO Eliminate copy
        let mut account = self.get(&inherent.target, Some(txn));

        // Apply inherent.
        let receipt = account_op(&mut account, inherent, receipt)?;

        // TODO Eliminate copy
        self.tree.put_batch(txn, &inherent.target, account);

        Ok(receipt)
    }

    fn prepare_receipts(receipts: &Receipts) -> (ReceiptsMap, ReceiptsMap, ReceiptsMap, Vec<&PrunedAccount>) {
        let mut sender_receipts = HashMap::new();
        let mut recipient_receipts = HashMap::new();
        let mut inherent_receipts = HashMap::new();
        let mut pruned_accounts = Vec::new();
        for receipt in &receipts.receipts {
            match receipt {
                Receipt::Transaction { index, sender, data } => {
                    if *sender {
                        sender_receipts.insert(*index, data);
                    } else {
                        recipient_receipts.insert(*index, data);
                    }
                },
                Receipt::Inherent { index, data } => {
                    inherent_receipts.insert(*index, data);
                },
                Receipt::PrunedAccount(ref account) => {
                    pruned_accounts.push(account);
                },
            }
        }
        (sender_receipts, recipient_receipts, inherent_receipts, pruned_accounts)
    }
}
