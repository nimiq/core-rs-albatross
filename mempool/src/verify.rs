use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::{account::AccountError, networks::NetworkId, transaction::TransactionError};
use nimiq_transaction::Transaction;
use parking_lot::RwLock;
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
    #[error("Transaction cannot be applied to sender account: {0}")]
    InvalidAccount(#[from] AccountError),
    #[error("Transaction already in mempool")]
    Known,
    #[error("Transaction is filtered")]
    Filtered,
    #[error("Can't verify transaction without consensus")]
    NoConsensus,
}

/// Verifies a transaction and adds it to the mempool.
pub(crate) fn verify_tx(
    mut transaction: Transaction,
    blockchain: Arc<RwLock<Blockchain>>,
    network_id: NetworkId,
    mempool_state: &Arc<RwLock<MempoolState>>,
    filter: Arc<RwLock<MempoolFilter>>,
    priority: TxPriority,
) -> Result<(), VerifyErr> {
    // 1. Verify transaction signature (and other stuff)
    transaction.verify_mut(network_id)?;

    // 2. Acquire blockchain read lock
    let blockchain = blockchain.read();

    // 3. Check validity window and already included
    let block_number = blockchain.block_number() + 1;
    if !transaction.is_valid_at(block_number) {
        return Err(VerifyErr::InvalidBlockNumber);
    }

    let hash: Blake2bHash = transaction.hash();
    if blockchain.contains_tx_in_validity_window(&hash.clone().into(), None) {
        return Err(VerifyErr::AlreadyIncluded);
    }

    // 4. Acquire the mempool state write lock
    let mut mempool_state = mempool_state.write();

    // 5. Check if we already know the transaction
    if mempool_state.contains(&hash) {
        // We already know this transaction, no need to process
        return Err(VerifyErr::Known);
    }

    // 6. Check if the transaction is going to be filtered.
    {
        let filter = filter.read();
        if !filter.accepts_transaction(&transaction) || filter.blacklisted(&hash) {
            // FIXME add transaction to blacklist
            return Err(VerifyErr::Filtered);
        }

        // TODO We also need to check:
        //  - filter.accepts_sender_balance()
        //  - filter.accepts_recipient_balance()
    }

    // 7. Add transaction to the mempool. Balance checks are performed within put().
    mempool_state.put(&blockchain, transaction, priority)?;

    Ok(())
}
