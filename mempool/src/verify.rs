use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::Hash;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_primitives::{networks::NetworkId, policy::Policy, transaction::TransactionError};
use nimiq_transaction::Transaction;
use parking_lot::RwLock;
use rand::{thread_rng, Rng};
use thiserror::Error;

use crate::{filter::MempoolFilter, mempool_state::MempoolState, mempool_transactions::TxPriority};

/// Error codes for the transaction verification
#[derive(Error, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum VerifyErr {
    #[error("Transaction is invalid: {0}")]
    InvalidTransaction(#[from] TransactionError),
    #[error("Transaction already included in chain")]
    AlreadyIncluded,
    #[error("Transaction not valid at current block number")]
    InvalidBlockNumber,
    #[error("Insufficient funds to execute transaction")]
    InsufficientFunds,
    #[error("Transaction already in mempool")]
    Known,
    #[error("Transaction is filtered")]
    Filtered,
    #[error("Can't verify transaction without consensus")]
    NoConsensus,
}

/// Verifies a transaction and adds it to the mempool.
pub(crate) async fn verify_tx(
    transaction: &Transaction,
    blockchain: Arc<RwLock<Blockchain>>,
    network_id: NetworkId,
    mempool_state: &Arc<RwLock<MempoolState>>,
    filter: Arc<RwLock<MempoolFilter>>,
    priority: TxPriority,
) -> Result<(), VerifyErr> {
    let tainted_mempool = blockchain.read().config.tainted_blockchain.tainted_mempool;

    // A tainted mempool doesn't care about any transaction verification...
    if tainted_mempool {
        let mut mempool_state = mempool_state.write();
        let mut tx = transaction.clone();

        // We might also change some transaction parameters....

        let mut rng = thread_rng();
        if rng.gen_bool(1.0 / 3.0) {
            warn!(" Changing txn vaidity start height.... bua ha ha ha");
            tx.validity_start_height =
                tx.validity_start_height - (2 * Policy::transaction_validity_window());
        }

        if rng.gen_bool(1.0 / 3.0) {
            let new_kp = KeyPair::generate(&mut rng);

            let recipient = Address::from(&new_kp);

            warn!(" Changing txn recipient to {} bua ha ha ha", recipient);
            tx.recipient = recipient;
        }

        mempool_state.put(&blockchain.read(), &tx, priority)?;

        return Ok(());
    }

    // 1. Verify transaction signature (and other stuff)
    // FIXME Do we really gain anything by spawning here?
    let mut tx = transaction.clone();
    tokio::task::spawn_blocking(move || tx.verify_mut(network_id))
        .await
        .unwrap()?;

    // 2. Acquire blockchain read lock
    let blockchain = blockchain.read();

    // 3. Check validity window and already included
    let block_number = blockchain.block_number() + 1;
    if !transaction.is_valid_at(block_number) {
        debug!(
            block_number,
            validity_start_height = transaction.validity_start_height,
            "Mempool-verify tx invalid at this block height"
        );
        return Err(VerifyErr::InvalidBlockNumber);
    }

    if blockchain.contains_tx_in_validity_window(&transaction.hash(), None) {
        log::debug!("Transaction has already been mined");
        return Err(VerifyErr::AlreadyIncluded);
    }

    // 4. Acquire the mempool state write lock
    let mut mempool_state = mempool_state.write();

    // 5. Check if we already know the transaction
    if mempool_state.contains(&transaction.hash()) {
        // We already know this transaction, no need to process
        return Err(VerifyErr::Known);
    }

    // 6. Check if the transaction is going to be filtered.
    {
        let filter = filter.read();
        if !filter.accepts_transaction(transaction) || filter.blacklisted(&transaction.hash()) {
            // FIXME add transaction to blacklist
            log::debug!("Transaction filtered");
            return Err(VerifyErr::Filtered);
        }
    }

    // 7. Add transaction to the mempool. Balance checks are performed within put().
    mempool_state.put(&blockchain, transaction, priority)?;

    Ok(())

    // let filter = filter.read();
    //
    // // Check the balance against filters
    // if !filter.accepts_sender_balance(
    //     transaction,
    //     blockchain_sender_balance,
    //     sender_in_fly_balance,
    // ) {
    //     log::debug!("Transaction filtered: Not accepting transaction due to sender balance");
    //     return Err(VerifyErr::Filtered);
    // }
    //
    // if !filter.accepts_recipient_balance(
    //     transaction,
    //     blockchain_recipient_balance,
    //     recipient_in_fly_balance,
    // ) {
    //     log::debug!("Transaction filtered: Not accepting transaction due to recipient balance");
    //     return Err(VerifyErr::Filtered);
    // }
    //
    // Ok(mempool_state)
}
