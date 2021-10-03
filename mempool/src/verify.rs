use std::sync::Arc;

use parking_lot::RwLock;

use nimiq_account::{Account, BasicAccount};
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_hash::Hash;
use nimiq_primitives::coin::Coin;

use nimiq_transaction::Transaction;

use crate::filter::MempoolFilter;
use crate::mempool::MempoolState;

/// Return code for the Mempool executor future
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReturnCode {
    /// Sender doesn't have enough funds.
    NotEnoughFunds,
    /// Transaction is invalid
    Invalid,
    /// Transaction is already known
    Known,
    /// Transaction is filtered
    Filtered,
    /// Transaction is accepted
    Accepted,
}

pub(crate) fn verify_tx(
    transaction: &Transaction,
    blockchain: Arc<RwLock<Blockchain>>,
    mempool_state: Arc<RwLock<MempoolState>>,
    filter: Arc<RwLock<MempoolFilter>>,
) -> ReturnCode {
    // Check if we already know the transaction
    let mempool_state = mempool_state.read();

    if mempool_state.contains(&transaction.hash()) {
        // We already know this transaction, no need to process
        log::debug!("Transaction is already known ");
        return ReturnCode::Known;
    }

    // Check if the transaction is going to be filtered.
    let filter = filter.read();

    if !filter.accepts_transaction(transaction) || filter.blacklisted(&transaction.hash()) {
        log::debug!("Transaction filtered");
        return ReturnCode::Filtered;
    }

    // 1. Acquire Blockchain read lock
    let blockchain = blockchain.read();

    // 2. Verify transaction signature (and other stuff)
    let network_id = blockchain.network_id();

    if let Err(err) = transaction.verify(network_id) {
        log::debug!("Intrinsic tx verification Failed {:?}", err);
        return ReturnCode::Invalid;
    }

    // 3. Check Validity Window and already included
    let block_height = blockchain.block_number() + 1;

    if !transaction.is_valid_at(block_height) {
        log::debug!("Transaction invalid at block {}", block_height);
        return ReturnCode::Invalid;
    }

    if blockchain.contains_tx_in_validity_window(&transaction.hash()) {
        log::debug!("Transaction has already been mined");
        return ReturnCode::Invalid;
    }

    // 4. Sequentialize per Sender to Check Balances and acquire the upgradable from the blockchain.
    // Perform all balances checks.
    let sender_account = match blockchain.get_account(&transaction.sender) {
        None => {
            log::debug!(
                "There is no account for this sender in the blockchain {}",
                transaction.sender.to_user_friendly_address()
            );
            return ReturnCode::Invalid;
        }
        Some(account) => account,
    };

    // Get recipient account to later check against filter rules.
    let recipient_account = match blockchain.get_account(&transaction.recipient) {
        None => Account::Basic(BasicAccount {
            balance: Coin::ZERO,
        }),
        Some(x) => x,
    };

    let blockchain_sender_balance = sender_account.balance();
    let blockchain_recipient_balance = recipient_account.balance();

    // Read the pending transactions balance
    let mut sender_current_balance = Coin::ZERO;
    let mut recipient_current_balance = blockchain_recipient_balance;

    if let Some(sender_state) = mempool_state.state_by_sender.get(&transaction.sender) {
        sender_current_balance = sender_state.total;
    }

    if let Some(recipient_state) = mempool_state.state_by_sender.get(&transaction.recipient) {
        // We found the recipient in the mempool. Subtract the mempool balance from the recipient balance
        recipient_current_balance -= recipient_state.total;
    }

    // Calculate the new balance assuming we add this transaction to the mempool
    let sender_in_fly_balance = transaction.total_value().unwrap() + sender_current_balance;
    let recipient_in_fly_balance = transaction.total_value().unwrap() + recipient_current_balance;

    // Check the balance against filters
    if !filter.accepts_sender_balance(
        transaction,
        blockchain_sender_balance,
        sender_in_fly_balance,
    ) {
        log::debug!("Transaction filtered: Not accepting transaction due to sender balance");
        return ReturnCode::Filtered;
    }

    if !filter.accepts_recipient_balance(
        transaction,
        blockchain_recipient_balance,
        recipient_in_fly_balance,
    ) {
        log::debug!("Transaction filtered: Not accepting transaction due to recipient balance");
        return ReturnCode::Filtered;
    }

    if sender_in_fly_balance > blockchain_sender_balance {
        log::debug!("Dropped because sum of txs in mempool is larger than the account balance");
        return ReturnCode::NotEnoughFunds;
    }

    ReturnCode::Accepted
}
