use std::collections::{BTreeSet, HashMap};

use hex;

use account::{Account, AccountError, AccountTransactionInteraction, AccountType, PrunedAccount, Receipt};
use beserial::Deserialize;
use database::{Environment, ReadTransaction, WriteTransaction};
use database as db;
use hash::Blake2bHash;
use keys::Address;
use network_primitives::networks::get_network_info;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use primitives::policy;
use transaction::{Transaction, TransactionFlags};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;

use crate::tree::AccountsTree;

#[derive(Debug)]
pub struct Accounts<'env> {
    env: &'env Environment,
    tree: AccountsTree<'env>,
}

impl<'env> Accounts<'env> {
    pub fn new(env: &'env Environment) -> Self {
        Accounts { env, tree: AccountsTree::new(env) }
    }

    pub fn init(&self, txn: &mut WriteTransaction, network_id: NetworkId) {
        let network_info = get_network_info(network_id).unwrap();
        let account_bytes = hex::decode(&network_info.genesis_accounts).unwrap();
        let reader = &mut &account_bytes[..];
        let count = u16::deserialize(reader).unwrap();

        for _ in 0..count {
            let address = Address::deserialize(reader).unwrap();
            let account = Account::deserialize(reader).unwrap();
            self.tree.put_batch(txn, &address, account);
        }
        self.tree.finalize_batch(txn);

        let genesis_header = &network_info.genesis_block.header;
        let genesis_body = network_info.genesis_block.body.as_ref().unwrap();
        self.commit(txn, &genesis_body.transactions, &genesis_body.miner, genesis_header.height)
            .expect("Failed to commit genesis block body");

        assert_eq!(self.tree.root_hash(txn), genesis_header.accounts_hash,
                   "Genesis AccountHash mismatch");
    }

    pub fn get(&self, address: &Address, txn_option: Option<&db::Transaction>) -> Account {
        match txn_option {
            Some(txn) => self.tree.get(txn, address),
            None => self.tree.get(&ReadTransaction::new(self.env), address)
        }.unwrap_or(Account::INITIAL)
    }

    pub fn get_chunk(&self, prefix: &str, size: usize, txn_option: Option<&db::Transaction>) -> Option<AccountsTreeChunk> {
        match txn_option {
            Some(txn) => self.tree.get_chunk(txn, prefix, size),
            None => self.tree.get_chunk(&ReadTransaction::new(self.env), prefix, size),
        }
    }

    pub fn get_accounts_proof(&self, txn: &db::Transaction, addresses: &[Address]) -> AccountsProof {
        self.tree.get_accounts_proof(txn, addresses)
    }

    pub fn hash(&self, txn_option: Option<&db::Transaction>) -> Blake2bHash {
        match txn_option {
            Some(txn) => self.tree.root_hash(txn),
            None => self.tree.root_hash(&ReadTransaction::new(self.env))
        }
    }

    pub fn hash_with(&self, transactions: &Vec<Transaction>, reward_address: &Address, block_height: u32) -> Result<Blake2bHash, AccountError> {
        let mut txn = WriteTransaction::new(self.env);

        self.commit(&mut txn, transactions, reward_address, block_height)?;
        let hash = self.hash(Some(&txn));

        txn.abort();
        Ok(hash)
    }

    pub fn commit(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, reward_address: &Address, block_height: u32) -> Result<Vec<Receipt>, AccountError> {
        let mut receipts = Vec::new();

        receipts.append(&mut self.process_senders(txn, transactions, block_height, HashMap::new(),
                             |account, transaction, block_height, _| account.commit_outgoing_transaction(transaction, block_height))?);

        receipts.append(&mut self.process_recipients(txn, transactions, block_height, HashMap::new(),
                                |account, transaction, block_height, _| account.commit_incoming_transaction(transaction, block_height))?);

        self.create_contracts(txn, transactions, block_height)?;

        receipts.append(&mut self.prune_accounts(txn, transactions)?);

        self.process_block_reward(txn, transactions, reward_address, block_height,
                                  |account, transaction, block_height, _| account.commit_incoming_transaction(transaction, block_height))?;

        self.tree.finalize_batch(txn);
        Ok(receipts)
    }

    pub fn revert(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, reward_address: &Address, block_height: u32, receipts: &Vec<Receipt>) -> Result<(), AccountError> {
        self.process_block_reward(txn, transactions, reward_address, block_height,
                                  |account, transaction, block_height, receipt| account.revert_incoming_transaction(transaction, block_height, receipt).map(|_| None))?;

        let (sender_receipts, recipient_receipts, pruned_accounts) = Self::prepare_receipts(receipts);

        self.restore_accounts(txn, pruned_accounts)?;

        self.revert_contracts(txn, transactions, block_height)?;

        self.process_recipients(txn, transactions, block_height, recipient_receipts,
                                |account, transaction, block_height, receipt| account.revert_incoming_transaction(transaction, block_height, receipt).map(|_| None))?;

        self.process_senders(txn, transactions, block_height, sender_receipts,
                             |account, transaction, block_height, receipt| account.revert_outgoing_transaction(transaction, block_height, receipt).map(|_| None))?;

        self.tree.finalize_batch(txn);
        Ok(())
    }

    pub fn collect_receipts(&self, transactions: &Vec<Transaction>, block_height: u32) -> Result<Vec<Receipt>, AccountError> {
        let mut txn = WriteTransaction::new(&self.env);
        let mut receipts = BTreeSet::new();

        self.process_senders(&mut txn, transactions, block_height, HashMap::new(),
                             |account, transaction, block_height, _| account.commit_outgoing_transaction(transaction, block_height))?
            .into_iter()
            .for_each(|receipt| { receipts.insert(receipt); () });

        self.process_recipients(&mut txn, transactions, block_height, HashMap::new(),
                                |account, transaction, block_height, _| account.commit_incoming_transaction(transaction, block_height))?
            .into_iter()
            .for_each(|receipt| { receipts.insert(receipt); () });

        self.create_contracts(&mut txn, transactions, block_height)?;

        self.prune_accounts(&mut txn, transactions)?
            .into_iter()
            .for_each(|receipt| { receipts.insert(receipt); () });

        txn.abort();
        Ok(receipts.into_iter().collect())
    }

    fn process_senders<F>(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, block_height: u32, mut receipts: HashMap<u16, &Vec<u8>>, account_op: F) -> Result<Vec<Receipt>, AccountError>
        where F: Fn(&mut Account, &Transaction, u32, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        let mut new_receipts = Vec::new();
        for (tx_idx, transaction) in transactions.iter().enumerate() {
            if let Some(data) = self.process_transaction(txn, &transaction.sender, Some(transaction.sender_type), transaction, block_height, receipts.remove(&(tx_idx as u16)), &account_op)? {
                new_receipts.push(Receipt::Transaction {
                    tx_idx: tx_idx as u16,
                    sender: true,
                    data,
                });
            }
        }
        Ok(new_receipts)
    }

    fn process_recipients<F>(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, block_height: u32, mut receipts: HashMap<u16, &Vec<u8>>, account_op: F) -> Result<Vec<Receipt>, AccountError>
        where F: Fn(&mut Account, &Transaction, u32, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        let mut new_receipts = Vec::new();
        for (tx_idx, transaction) in transactions.iter().enumerate() {
            // FIXME This doesn't check that account_type == transaction.recipient_type when reverting
            let recipient_type = if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                None
            } else {
                Some(transaction.recipient_type)
            };

            if let Some(data) = self.process_transaction(txn, &transaction.recipient, recipient_type, transaction, block_height, receipts.remove(&(tx_idx as u16)), &account_op)? {
                new_receipts.push(Receipt::Transaction {
                    tx_idx: tx_idx as u16,
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

    fn process_block_reward<F>(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, reward_address: &Address, block_height: u32, account_op: F) -> Result<(), AccountError>
        where F: Fn(&mut Account, &Transaction, u32, Option<&Vec<u8>>) -> Result<Option<Vec<u8>>, AccountError> {

        // Sum up transaction fees.
        let mut fees = policy::block_reward_at(block_height);
        for tx in transactions {
            fees = Account::balance_add(fees, tx.fee)?;
        }

        // "Coinbase" transaction.
        let coinbase_tx = Transaction::new_basic(
            Address::from([0u8; Address::SIZE]),
            reward_address.clone(),
            fees,
            Coin::ZERO,
            block_height,
            NetworkId::Main, // XXX ignored
        );

        // Coinbase transaction must not produce a receipt.
        match self.process_transaction(txn, reward_address, Some(AccountType::Basic), &coinbase_tx, block_height, None, &account_op)? {
            Some(_) => Err(AccountError::InvalidForRecipient),
            None => Ok(())
        }
    }

    fn create_contracts(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, block_height: u32) -> Result<(), AccountError> {
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

    fn revert_contracts(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>, block_height: u32) -> Result<(), AccountError> {
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

    fn prune_accounts(&self, txn: &mut WriteTransaction, transactions: &Vec<Transaction>) -> Result<Vec<Receipt>, AccountError> {
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

    fn prepare_receipts(receipts: &Vec<Receipt>) -> (HashMap<u16, &Vec<u8>>, HashMap<u16, &Vec<u8>>, Vec<&PrunedAccount>) {
        let mut sender_receipts = HashMap::new();
        let mut recipient_receipts = HashMap::new();
        let mut pruned_accounts = Vec::new();
        for receipt in receipts {
            match receipt {
                Receipt::Transaction { tx_idx, sender, data } => {
                    if *sender {
                        sender_receipts.insert(*tx_idx, data);
                    } else {
                        recipient_receipts.insert(*tx_idx, data);
                    }
                },
                Receipt::PrunedAccount(ref account) => {
                    pruned_accounts.push(account);
                }
            }
        }
        (sender_receipts, recipient_receipts, pruned_accounts)
    }
}
