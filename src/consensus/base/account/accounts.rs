use consensus::base::account::{Account, AccountError, AccountType};
use consensus::base::block::{Block, BlockBody};
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::Blake2bHash;
use consensus::base::transaction::{Transaction, TransactionFlags};
use consensus::networks::NetworkId;
use consensus::policy;
use std::collections::HashMap;
use super::tree::AccountsTree;
use utils::db;
use utils::db::{Environment, ReadTransaction, WriteTransaction};
use consensus::base::primitive::coin::Coin;

#[derive(Debug)]
pub struct Accounts<'env> {
    env: &'env Environment,
    tree: AccountsTree<'env>,
}

impl<'env> Accounts<'env> {
    pub fn new(env: &'env Environment) -> Self {
        return Accounts { env, tree: AccountsTree::new(env) };
    }

    pub fn get(&self, address: &Address, txn_option: Option<&db::Transaction>) -> Account {
        return match txn_option {
            Some(txn) => self.tree.get(txn, address),
            None => self.tree.get(&ReadTransaction::new(self.env), address)
        }.unwrap_or(Account::INITIAL);
    }

    pub fn hash(&self, txn_option: Option<&db::Transaction>) -> Blake2bHash {
        return match txn_option {
            Some(txn) => self.tree.root_hash(txn),
            None => self.tree.root_hash(&ReadTransaction::new(self.env))
        };
    }

    pub fn commit_block(&self, txn: &mut WriteTransaction, block: &Block) -> Result<(), AccountError> {
        unimplemented!();
    }

    pub fn revert_block(&self, txn: &mut WriteTransaction, block: &Block) -> Result<(), AccountError> {
        unimplemented!();
    }

    pub fn commit_block_body(&self, txn: &mut WriteTransaction, body: &BlockBody, block_height: u32) -> Result<(), AccountError> {
        // Process sender accounts.
        for transaction in &body.transactions {
            self.process_transaction(txn, &transaction.sender, transaction, block_height,
                                     |account, transaction, block_height| account.with_outgoing_transaction(transaction, block_height))?;
        }

        // Process recipient accounts.
        for transaction in &body.transactions {
            self.process_transaction(txn, &transaction.recipient, transaction, block_height,
                                     |account, transaction, block_height| account.with_incoming_transaction(transaction, block_height))?;
        }

        // Create contracts.
        for transaction in &body.transactions {
            self.create_contract(txn, transaction, block_height)?;
        }

        self.prune_accounts(txn, body)?;

        self.process_miner_reward(txn, body, block_height,
                                  |account, transaction, block_height| account.with_incoming_transaction(transaction, block_height))?;

        self.tree.finalize_batch(txn);
        return Ok(());
    }

    pub fn revert_block_body(&self, txn: &mut WriteTransaction, body: &BlockBody, block_height: u32) -> Result<(), AccountError> {
        self.process_miner_reward(txn, body, block_height,
                                  |account, transaction, block_height| account.without_incoming_transaction(transaction, block_height))?;

        // Restore pruned accounts.
        self.restore_accounts(txn, body)?;

        // Revert created contracts.
        for transaction in &body.transactions {
            self.revert_contract(txn, transaction, block_height)?;
        }

        // Process recipient accounts.
        for transaction in &body.transactions {
            self.process_transaction(txn, &transaction.recipient, transaction, block_height,
                                     |account, transaction, block_height| account.without_incoming_transaction(transaction, block_height))?;
        }

        // Process sender accounts.
        for transaction in &body.transactions {
            self.process_transaction(txn, &transaction.sender, transaction, block_height,
                                     |account, transaction, block_height| account.without_outgoing_transaction(transaction, block_height))?;
        }

        self.tree.finalize_batch(txn);
        return Ok(());
    }

    fn process_transaction<F>(&self, txn: &mut WriteTransaction, address: &Address, transaction: &Transaction, block_height: u32, account_op: F) -> Result<(), AccountError>
        where F: Fn(Account, &Transaction, u32) -> Result<Account, AccountError> {

        let account = self.get(address, Some(txn));
        let new_account = account_op(account, transaction, block_height)?;
        self.tree.put_batch(txn, address, new_account);
        return Ok(());
    }

    fn process_miner_reward<F>(&self, txn: &mut WriteTransaction, body: &BlockBody, block_height: u32, account_op: F) -> Result<(), AccountError>
        where F: Fn(Account, &Transaction, u32) -> Result<Account, AccountError> {

        // Sum up transaction fees.
        let mut fees = policy::block_reward_at(block_height);
        for tx in &body.transactions {
            fees = Account::balance_add(fees, tx.fee)?;
        }

        // "Coinbase" transaction.
        let coinbase_tx = Transaction::new_basic(
            Address::from([0u8; Address::SIZE]),
            body.miner.clone(),
            fees,
            Coin::ZERO,
            block_height,
            NetworkId::Main, // XXX ignored
        );

        return self.process_transaction(txn, &body.miner, &coinbase_tx, block_height, account_op);
    }

    fn create_contract(&self, txn: &mut WriteTransaction, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            return Ok(());
        }

        let recipient_account = self.get(&transaction.recipient, Some(txn));
        if recipient_account.account_type() != AccountType::Basic {
            return Err(AccountError("Contract already exists".to_string()));
        }

        let new_recipient_account = Account::new_contract(transaction.recipient_type, recipient_account.balance(), transaction, block_height)?;
        self.tree.put_batch(txn, &transaction.recipient, new_recipient_account);
        return Ok(());
    }

    fn revert_contract(&self, txn: &mut WriteTransaction, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            return Ok(());
        }

        let recipient_account = self.get(&transaction.recipient, Some(txn));
        if recipient_account.account_type() == AccountType::Basic {
            return Err(AccountError("Contract does not exist".to_string()));
        }

        if recipient_account.account_type() != transaction.recipient_type {
            return Err(AccountError("Wrong contract type".to_string()));
        }

        let new_recipient_account = Account::new_basic(recipient_account.balance());
        self.tree.put_batch(txn, &transaction.recipient, new_recipient_account);
        return Ok(());
    }

    fn prune_accounts(&self, txn: &mut WriteTransaction, body: &BlockBody) -> Result<(), AccountError> {
        let mut pruned_accounts: HashMap<Address, Account> = HashMap::new();
        for pruned_account in &body.pruned_accounts {
            pruned_accounts.insert(pruned_account.address.clone(), pruned_account.account.clone());
        }

        for transaction in &body.transactions {
            let sender_account = self.get(&transaction.sender, Some(txn));
            if !sender_account.is_to_be_pruned() {
                continue;
            }

            let correctly_pruned = match pruned_accounts.get(&transaction.sender) {
                Some(pruned_account) => *pruned_account == sender_account,
                None => false
            };
            if !correctly_pruned {
                return Err(AccountError("Account not pruned correctly".to_string()));
            }

            self.tree.put_batch(txn, &transaction.sender, Account::INITIAL);
            pruned_accounts.remove(&transaction.sender);
        }

        if pruned_accounts.len() > 0 {
            return Err(AccountError("Account was invalidly pruned".to_string()));
        }

        return Ok(());
    }

    fn restore_accounts(&self, txn: &mut WriteTransaction, body: &BlockBody) -> Result<(), AccountError> {
        for pruned_account in &body.pruned_accounts {
            self.tree.put_batch(txn, &pruned_account.address, pruned_account.account.clone());
        }
        return Ok(());
    }
}
