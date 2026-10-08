use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    networks::NetworkId,
    transaction::TransactionError,
};
use nimiq_transaction::{account::staking_contract::IncomingStakingTransactionData, Transaction};
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
    #[error("Sender has reached the maximum number of transactions in the mempool")]
    SenderLimitReached,
    #[error("Transaction uses a small-order (wildcard) Ed25519 key")]
    SmallOrderKey,
    #[error("Transaction targets a validator that does not exist")]
    UnknownValidator,
}

/// Verifies a transaction and adds it to the mempool.
pub(crate) fn verify_tx(
    mut transaction: Transaction,
    blockchain: Arc<RwLock<Blockchain>>,
    protocol_version: u16,
    network_id: NetworkId,
    mempool_state: &Arc<RwLock<MempoolState>>,
    filter: Arc<RwLock<MempoolFilter>>,
    priority: TxPriority,
) -> Result<(), VerifyErr> {
    let hash: Blake2bHash = transaction.hash();

    // 1. Cheap stateful checks first, under read locks. Signature and proof verification can cost
    //    tens of milliseconds, so nothing that would be refused anyway should reach it.
    {
        let blockchain = blockchain.read();
        check_chain_state(&blockchain, &transaction, &hash)?;

        let mempool_state = mempool_state.read();
        if mempool_state.contains(&hash) {
            return Err(VerifyErr::Known);
        }

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

        mempool_state.check_put(&blockchain, &transaction)?;
        check_validator_exists(&blockchain, &transaction)?;
    }

    // 2. Verify transaction signature (and other stuff) using the cached protocol version.
    transaction.verify_mut(network_id, protocol_version)?;

    // 3. Add transaction to the mempool. The chain may have moved meanwhile, so the chain checks
    //    run again and put() repeats the balance checks under the write lock.
    let blockchain = blockchain.read();
    check_chain_state(&blockchain, &transaction, &hash)?;
    let mut mempool_state = mempool_state.write();
    mempool_state.put(&blockchain, transaction, priority)?;

    Ok(())
}

/// Validity window and already-included checks against the current chain state.
fn check_chain_state(
    blockchain: &Blockchain,
    transaction: &Transaction,
    hash: &Blake2bHash,
) -> Result<(), VerifyErr> {
    let block_number = blockchain.block_number() + 1;
    if !transaction.is_valid_at(block_number) {
        return Err(VerifyErr::InvalidBlockNumber);
    }
    if blockchain.contains_tx_in_validity_window(&hash.clone().into(), None) {
        return Err(VerifyErr::AlreadyIncluded);
    }
    Ok(())
}

/// Refuses an `UpdateValidator` for a validator that does not exist: it would fail on chain
/// anyway, and its proof of knowledge is the most expensive thing the mempool verifies.
fn check_validator_exists(
    blockchain: &Blockchain,
    transaction: &Transaction,
) -> Result<(), VerifyErr> {
    if transaction.recipient_type != AccountType::Staking {
        return Ok(());
    }
    // Malformed data is rejected by the stateless verification that follows.
    let Ok(IncomingStakingTransactionData::UpdateValidator { proof, .. }) =
        IncomingStakingTransactionData::parse(transaction)
    else {
        return Ok(());
    };
    let validator_address = proof.compute_signer();

    let staking_contract = blockchain
        .get_staking_contract_if_complete(None)
        .ok_or(VerifyErr::NoConsensus)?;
    let data_store = blockchain.get_staking_contract_store();
    let db_txn = blockchain.read_transaction();
    if staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .is_none()
    {
        return Err(VerifyErr::UnknownValidator);
    }
    Ok(())
}
