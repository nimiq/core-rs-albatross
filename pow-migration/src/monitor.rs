use std::ops::Range;

use log::{error, info};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_rpc::{
    primitives::{OutgoingTransaction, TransactionDetails},
    Client,
};
use percentage::Percentage;
use thiserror::Error;

use crate::{async_retryer, types::GenesisValidator};

/// Readiness state of all of the validators registered in the PoW chain
pub enum ValidatorsReadiness {
    /// Validators are not ready.
    /// Encodes the stake that is ready in the inner type.
    NotReady(Coin),
    /// Validators are ready.
    /// Encodes the stake that is ready in the inner type.
    Ready(Coin),
}

/// Error types that can be returned by the monitor
#[derive(Error, Debug)]
pub enum Error {
    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(#[from] nimiq_rpc::jsonrpsee::core::ClientError),
}

/// Stake percentage that is considered to indicate that the validators are ready
pub const READY_PERCENTAGE: u8 = 80;

/// Sends a transaction to the Nimiq PoW chain to report that we are ready
/// The transaction format is defined as follows:
/// - Sender: Validator address
/// - Recipient: Burn address
/// - Value: 1 Luna
/// - Data: Hash of the generated `GenesisConfig`
///
pub fn generate_ready_tx(validator: String, hash: &Blake2bHash) -> OutgoingTransaction {
    info!(
        validator_address = validator,
        pos_genesis_hash = %hash,
        "Generating ready transaction"
    );
    OutgoingTransaction {
        from: validator,
        to: Address::burn_address().to_user_friendly_address(),
        value: 1, //Lunas
        fee: 0,
        data: Some(hash.to_hex()),
    }
}

/// Sends a transaction to the Nimiq PoW chain to indicate that we are online
/// The transaction format is defined as follows:
/// - Sender: Validator address
/// - Recipient: Burn address
/// - Value: 1 Luna
/// - Data: "online"
pub fn generate_online_tx(validator: String) -> OutgoingTransaction {
    log::debug!(
        validator_address = validator,
        "Generating online transaction"
    );
    OutgoingTransaction {
        from: validator,
        to: Address::burn_address().to_user_friendly_address(),
        value: 1, //Lunas
        fee: 0,
        data: Some("online".to_string()),
    }
}

/// Checks if we have seen an online transaction from a validator in the specified range
pub async fn get_online_txns(
    pow_client: &Client,
    validator: String,
    block_window: Range<u32>,
) -> Vec<TransactionDetails> {
    let Ok(transactions) =
        async_retryer(|| pow_client.get_transactions_by_address(&validator, 10)).await
    else {
        return vec![];
    };

    transactions
        .into_iter()
        .filter(|txn| is_valid_online_txn(txn, &block_window))
        .collect()
}

/// Checks if we have seen a ready transaction from a validator in the specified range
pub async fn get_ready_txns(
    pow_client: &Client,
    validator: String,
    block_window: Range<u32>,
    pos_genesis_config_hash: &Blake2bHash,
) -> Vec<TransactionDetails> {
    if let Ok(transactions) =
        async_retryer(|| pow_client.get_transactions_by_address(&validator, 10)).await
    {
        let genesis_config_hash_hex = pos_genesis_config_hash.to_hex();

        let filtered_txns: Vec<TransactionDetails> = transactions
            .into_iter()
            .filter(|txn| is_valid_ready_txn(txn, &block_window, &genesis_config_hash_hex))
            .collect();
        return filtered_txns;
    }

    Vec::new()
}

/// Checks if the provided transaction meets the criteria in order to be
/// considered a valid ready-transaction
fn is_valid_ready_txn(
    txn: &TransactionDetails,
    block_window: &Range<u32>,
    genesis_config_hash: &String,
) -> bool {
    // Check if the txn contains extra data and matches our genesis config hash
    Some(genesis_config_hash) == txn.data.as_ref()
        && block_window.contains(&txn.block_number)
        && txn.to_address == Address::burn_address().to_user_friendly_address()
}

/// Checks if the provided transaction meets the criteria in order to be
/// considered a valid online-transaction
fn is_valid_online_txn(txn: &TransactionDetails, block_window: &Range<u32>) -> bool {
    Some("online".to_string()) == txn.data
        && block_window.contains(&txn.block_number)
        && txn.to_address == Address::burn_address().to_user_friendly_address()
}

/// Sends a transaction into the Nimiq PoW chain
pub async fn send_tx(client: &Client, transaction: OutgoingTransaction) -> Result<(), Error> {
    match client.send_transaction(&transaction).await {
        Ok(_) => {
            info!("Sent transaction to the Nimiq PoW network");
            Ok(())
        }
        Err(error) => {
            error!(?error, "Failed sending transaction");
            Err(Error::Rpc(error))
        }
    }
}

/// Returns true if the given validator was ready at the given activation block window
pub async fn was_validator_ready(
    pow_client: &Client,
    validator_address: String,
    activation_block_window: Range<u32>,
    genesis_hash: String,
) -> bool {
    if let Ok(transactions) =
        async_retryer(|| pow_client.get_transactions_by_address(&validator_address, 10)).await
    {
        // We only keep the ones past the activation window that met the activation criteria
        let filtered_txns: Vec<TransactionDetails> = transactions
            .into_iter()
            .filter(|txn| is_valid_ready_txn(txn, &activation_block_window, &genesis_hash))
            .collect();
        if !filtered_txns.is_empty() {
            return true;
        }
    }
    false
}

/// Checks if enough validators are ready.
/// If thats the case, the number of slots which are ready are returned.
pub async fn check_validators_ready(
    pow_client: &Client,
    validators: &Vec<GenesisValidator>,
    activation_block_window: Range<u32>,
    pos_genesis_config_hash: &Blake2bHash,
) -> ValidatorsReadiness {
    // First calculate the total amount of stake
    let total_stake: Coin = validators
        .iter()
        .map(|validator| validator.total_stake)
        .sum();

    log::debug!(registered_stake = %total_stake);

    let mut ready_validators = Vec::new();
    let genesis_config_hash_hex = pos_genesis_config_hash.to_hex();

    log::info!("Starting to collect transactions from validators...");

    // Now we need to collect all the transactions for each validator
    for validator in validators {
        let address = validator
            .validator
            .validator_address
            .to_user_friendly_address();
        if let Ok(transactions) =
            async_retryer(|| pow_client.get_transactions_by_address(&address, 10)).await
        {
            info!(
                num_transactions = transactions.len(),
                from_address = address,
                "Transactions found for validator"
            );
            // We only keep the ones past the activation window that met the activation criteria
            let filtered_txns: Vec<TransactionDetails> = transactions
                .into_iter()
                .filter(|txn| {
                    is_valid_ready_txn(txn, &activation_block_window, &genesis_config_hash_hex)
                })
                .collect();
            info!(
                num_transactions = filtered_txns.len(),
                "Transactions that met the readiness criteria",
            );
            if !filtered_txns.is_empty() {
                ready_validators.push(validator);
            }
        }
    }

    // Now we need to see if we have enough stake ready
    let mut ready_stake = Coin::ZERO;

    for ready_validator in ready_validators {
        ready_stake += ready_validator.total_stake;

        info!(
            address = ready_validator
                .validator
                .validator_address
                .to_user_friendly_address(),
            stake = %ready_validator.total_stake,
            "Validator is ready",
        );
    }

    let percent = Percentage::from(READY_PERCENTAGE);
    let needed_stake = percent.apply_to(u64::from(total_stake));

    info!(
        needed_stake,
        stake_ready = u64::from(ready_stake),
        "Stake needed vs ready"
    );

    if u64::from(ready_stake) >= needed_stake {
        info!("Enough validators are ready to start the PoS Chain!");
        ValidatorsReadiness::Ready(ready_stake)
    } else {
        info!(needed_stake, "Not enough validators are ready");
        ValidatorsReadiness::NotReady(ready_stake)
    }
}
