use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use std::{
    fmt::{self, Display, Formatter},
    sync::Arc,
};

use nimiq_account::{
    Account, AccountTransactionInteraction, BasicAccount, BlockState, ReservedBalance,
    StakingContract,
};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::Hash;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};

use nimiq_transaction::Transaction;

use crate::filter::MempoolFilter;
use crate::mempool_state::MempoolState;

/// Return codes for transaction signature verification
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignVerifReturnCode {
    /// Transaction signature is invalid
    Invalid,
    /// Transaction signature is correct
    SignOk,
}

/// Error codes for the transaction verification
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerifyErr {
    /// Sender doesn't have enough funds.
    NotEnoughFunds,
    /// Transaction signature is invalid
    InvalidSignature,
    /// Transaction not valid for the validation window
    InvalidTxWindow,
    /// Transaction not valid for the current block height
    InvalidBlockHeight,
    /// Transaction sender doesn't exist
    InvalidSender,
    /// Transaction is already known
    Known,
    /// Transaction is filtered
    Filtered,
    /// Transaction cannot succeed
    CannotSucceed,
    /// Fee can not be paid
    CantPayFee,
}

impl Display for VerifyErr {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            VerifyErr::NotEnoughFunds => {
                write!(f, "Not enough funds")
            }
            VerifyErr::InvalidSignature => {
                write!(f, "Invalid signature")
            }
            VerifyErr::InvalidTxWindow => {
                write!(f, "Invalid transaction window")
            }
            VerifyErr::InvalidBlockHeight => {
                write!(f, "Invalid block height")
            }
            VerifyErr::InvalidSender => {
                write!(f, "Invalid sender")
            }
            VerifyErr::Known => {
                write!(f, "Known")
            }
            VerifyErr::Filtered => {
                write!(f, "Filtered")
            }
            VerifyErr::CannotSucceed => {
                write!(f, "Cannot succeed")
            }
            VerifyErr::CantPayFee => {
                write!(f, "Cant pay fee")
            }
        }
    }
}

/// Verifies a Transaction
///
/// This function takes a reference to a RW Lock of the mempool_state and
/// returns a result of a RwLockUpgradableReadGuard of the mempool such that in
/// case of an accepted transaction (`Ok(RwLockUpgradableReadGuard)`), the
/// caller can upgrade the lock and add the transaction to the mempool.
pub(crate) async fn verify_tx<'a>(
    transaction: &Transaction,
    blockchain: Arc<RwLock<Blockchain>>,
    network_id: Arc<NetworkId>,
    mempool_state: &'a Arc<RwLock<MempoolState>>,
    filter: Arc<RwLock<MempoolFilter>>,
) -> Result<RwLockUpgradableReadGuard<'a, MempoolState>, VerifyErr> {
    // 1. Verify transaction signature (and other stuff)
    let mut tx = transaction.clone();

    let sign_verification_handle = tokio::task::spawn_blocking(move || {
        if let Err(err) = tx.verify_mut(*network_id) {
            log::debug!("Intrinsic tx verification Failed {:?}", err);
            return SignVerifReturnCode::Invalid;
        }
        SignVerifReturnCode::SignOk
    });

    // Check the result of the sign verification for the tx
    match sign_verification_handle.await {
        Ok(rc) => {
            if rc == SignVerifReturnCode::Invalid {
                // If signature verification failed we just return
                return Err(VerifyErr::InvalidSignature);
            }
        }
        Err(_err) => {
            return Err(VerifyErr::InvalidSignature);
        }
    };

    // 2. Acquire the mempool state upgradable read lock
    let blockchain = blockchain.read();
    let mempool_state = mempool_state.upgradable_read();

    // 3. Check if we already know the transaction
    if mempool_state.contains(&transaction.hash()) {
        // We already know this transaction, no need to process
        return Err(VerifyErr::Known);
    }

    // 4. Check if the transaction is going to be filtered.
    {
        let filter = filter.read();
        if !filter.accepts_transaction(transaction) || filter.blacklisted(&transaction.hash()) {
            log::debug!("Transaction filtered");
            return Err(VerifyErr::Filtered);
        }
    }

    // 5. Acquire Blockchain read lock

    // 6. Check Validity Window and already included
    let block_height = blockchain.block_number() + 1;

    if !transaction.is_valid_at(block_height) {
        debug!(
            block_height = block_height,
            validity_start_height = transaction.validity_start_height,
            "Mempool-verify tx invalid at this block height"
        );
        return Err(VerifyErr::InvalidBlockHeight);
    }

    if blockchain.contains_tx_in_validity_window(&transaction.hash(), None) {
        log::debug!("Transaction has already been mined");
        return Err(VerifyErr::InvalidTxWindow);
    }

    // 7. Sequentialize per Sender to Check Balances and acquire the upgradable from the blockchain.
    //    Perform all balances checks.
    let sender_account = match blockchain.get_account(&transaction.sender).or_else(|| {
        if transaction.total_value() != Coin::ZERO {
            None
        } else {
            Some(Account::Basic(BasicAccount {
                balance: Coin::ZERO,
            }))
        }
    }) {
        None => {
            log::debug!(
                "There is no account for this sender in the blockchain {}",
                transaction.sender.to_user_friendly_address()
            );
            return Err(VerifyErr::InvalidSender);
        }
        Some(account) => account,
    };

    // 8. Get recipient account to later check against filter rules.
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

    let mut reserved_balance = ReservedBalance::new(transaction.sender.clone());

    if let Some(sender_state) = mempool_state.state_by_sender.get(&transaction.sender) {
        sender_current_balance = sender_state.reserved_balance.balance();
        reserved_balance = sender_state.reserved_balance.clone();
    }

    if let Some(recipient_state) = mempool_state.state_by_sender.get(&transaction.recipient) {
        // We found the recipient in the mempool. Subtract the mempool balance from the recipient balance
        recipient_current_balance -= recipient_state.reserved_balance.balance();
    }

    // The sender must be able to at least pay the fee (in case the tx fails) assuming all pending txns in the mempool for this sender are included in a block
    blockchain
        .reserve_balance(&sender_account, transaction, &mut reserved_balance)
        .map_err(|_| VerifyErr::CantPayFee)?;

    // FIXME Store the reserved_balance in state_by_sender!!!

    // 9. Drop the blockchain lock since it is no longer needed
    drop(blockchain);

    // Calculate the new balance assuming we add this transaction to the mempool
    let sender_in_fly_balance = transaction.total_value() + sender_current_balance;
    let recipient_in_fly_balance = transaction.total_value() + recipient_current_balance;

    let filter = filter.read();

    // Check the balance against filters
    if !filter.accepts_sender_balance(
        transaction,
        blockchain_sender_balance,
        sender_in_fly_balance,
    ) {
        log::debug!("Transaction filtered: Not accepting transaction due to sender balance");
        return Err(VerifyErr::Filtered);
    }

    if !filter.accepts_recipient_balance(
        transaction,
        blockchain_recipient_balance,
        recipient_in_fly_balance,
    ) {
        log::debug!("Transaction filtered: Not accepting transaction due to recipient balance");
        return Err(VerifyErr::Filtered);
    }

    if sender_in_fly_balance > blockchain_sender_balance {
        log::debug!(
            "Dropped because sum of txs in mempool {} is larger than the account balance {}",
            sender_in_fly_balance,
            blockchain_sender_balance
        );
        return Err(VerifyErr::NotEnoughFunds);
    }

    Ok(mempool_state)
}
