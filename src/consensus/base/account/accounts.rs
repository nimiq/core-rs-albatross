use beserial::Deserialize;
use hex;
use std::collections::HashMap;
use crate::consensus::base::account::{Account, AccountType, AccountError};
use crate::consensus::base::account::tree::AccountsTree;
use crate::consensus::base::block::{Block, BlockBody};
use crate::consensus::base::primitive::{Address, Coin};
use hash::Blake2bHash;
use crate::consensus::base::transaction::{Transaction, TransactionFlags};
use crate::consensus::networks::{NetworkId, get_network_info};
use crate::consensus::policy;
use database::{Environment, ReadTransaction, WriteTransaction};
use database as db;

#[derive(Debug)]
pub struct Accounts<'env> {
    env: &'env Environment,
    tree: AccountsTree<'env>,
}

impl<'env> Accounts<'env> {
    pub fn new(env: &'env Environment) -> Self {
        return Accounts { env, tree: AccountsTree::new(env) };
    }

    pub fn init(&self, txn: &mut WriteTransaction, network_id: NetworkId) {
        let network_info = get_network_info(network_id).unwrap();
        let account_bytes = hex::decode(&network_info.genesis_accounts).unwrap();
        let reader = &mut &account_bytes[..];
        let count = u16::deserialize(reader).unwrap();

        for i in 0..count {
            let address = Address::deserialize(reader).unwrap();
            let account = Account::deserialize(reader).unwrap();
            self.tree.put_batch(txn, &address, account);
        }
        self.tree.finalize_batch(txn);

        let genesis_header = &network_info.genesis_block.header;
        let genesis_body = network_info.genesis_block.body.as_ref().unwrap();
        self.commit_block_body(txn, genesis_body, genesis_header.height)
            .expect("Failed to commit genesis block body");

        assert_eq!(self.tree.root_hash(txn), genesis_header.accounts_hash,
                   "Genesis AccountHash mismatch");
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

    pub fn hash_with_block_body(&self, body: &BlockBody, block_height: u32) -> Result<Blake2bHash, AccountError> {
        let mut txn = WriteTransaction::new(self.env);

        self.commit_block_body(&mut txn, body, block_height)?;

        let hash = self.hash(Some(&txn));
        txn.abort();
        Ok(hash)
    }

    pub fn commit_block(&self, txn: &mut WriteTransaction, block: &Block) -> Result<(), AccountError> {
        assert!(block.body.is_some(), "Cannot commit block without body");

        self.commit_block_body(txn, block.body.as_ref().unwrap(), block.header.height)?;

        if block.header.accounts_hash != self.tree.root_hash(txn) {
            return Err(AccountError::AccountsHashMismatch);
        }

        return Ok(());
    }

    pub fn revert_block(&self, txn: &mut WriteTransaction, block: &Block) -> Result<(), AccountError> {
        assert!(block.body.is_some(), "Cannot revert block without body");

        if block.header.accounts_hash != self.tree.root_hash(txn) {
            return Err(AccountError::AccountsHashMismatch);
        }

        return Ok(self.revert_block_body(txn, block.body.as_ref().unwrap(), block.header.height)?);
    }

    pub fn commit_block_body(&self, txn: &mut WriteTransaction, body: &BlockBody, block_height: u32) -> Result<(), AccountError> {
        // Process sender accounts.
        for transaction in &body.transactions {
            self.process_transaction(txn, &transaction.sender, Some(transaction.sender_type), transaction, block_height,
                                     |account, transaction, block_height| account.with_outgoing_transaction(transaction, block_height))?;
        }

        // Process recipient accounts.
        for transaction in &body.transactions {
            let recipient_type = match transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                true => None,
                false => Some(transaction.recipient_type)
            };
            self.process_transaction(txn, &transaction.recipient, recipient_type, transaction, block_height,
                                     |account, transaction, block_height| account.with_incoming_transaction(transaction, block_height))?;
        }

        // Create contracts.
        for transaction in &body.transactions {
            if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                self.create_contract(txn, transaction, block_height)?;
            }
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
            if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                self.revert_contract(txn, transaction, block_height)?;
            }
        }

        // Process recipient accounts.
        for transaction in &body.transactions {
            let recipient_type = match transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                true => None,
                false => Some(transaction.recipient_type)
            };
            self.process_transaction(txn, &transaction.recipient, recipient_type, transaction, block_height,
                                     |account, transaction, block_height| account.without_incoming_transaction(transaction, block_height))?;
        }

        // Process sender accounts.
        for transaction in &body.transactions {
            self.process_transaction(txn, &transaction.sender, Some(transaction.sender_type), transaction, block_height,
                                     |account, transaction, block_height| account.without_outgoing_transaction(transaction, block_height))?;
        }

        self.tree.finalize_batch(txn);
        return Ok(());
    }

    fn process_transaction<F>(&self, txn: &mut WriteTransaction, address: &Address, account_type: Option<AccountType>, transaction: &Transaction, block_height: u32, account_op: F) -> Result<(), AccountError>
        where F: Fn(Account, &Transaction, u32) -> Result<Account, AccountError> {

        let account = self.get(address, Some(txn));

        // Check account type.
        if account_type.is_some() && account.account_type() != account_type.unwrap() {
            return Err(AccountError::TypeMismatch);
        }

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

        return self.process_transaction(txn, &body.miner, Some(AccountType::Basic), &coinbase_tx, block_height, account_op);
    }

    fn create_contract(&self, txn: &mut WriteTransaction, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        assert!(transaction.flags.contains(TransactionFlags::CONTRACT_CREATION));

        let recipient_account = self.get(&transaction.recipient, Some(txn));
        let new_recipient_account = Account::new_contract(transaction.recipient_type, recipient_account.balance(), transaction, block_height)?;
        self.tree.put_batch(txn, &transaction.recipient, new_recipient_account);
        return Ok(());
    }

    fn revert_contract(&self, txn: &mut WriteTransaction, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        assert!(transaction.flags.contains(TransactionFlags::CONTRACT_CREATION));

        let recipient_account = self.get(&transaction.recipient, Some(txn));
        if recipient_account.account_type() != transaction.recipient_type {
            return Err(AccountError::TypeMismatch);
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
                return Err(AccountError::InvalidPruning);
            }

            self.tree.put_batch(txn, &transaction.sender, Account::INITIAL);
            pruned_accounts.remove(&transaction.sender);
        }

        if pruned_accounts.len() > 0 {
            return Err(AccountError::InvalidPruning);
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
