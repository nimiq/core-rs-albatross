pub mod types;

use std::ops::Range;

use log::{error, info};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_rpc::{
    primitives::{OutgoingTransaction, TransactionDetails},
    Client,
};
use percentage::Percentage;

use crate::{
    monitor::types::{Error, ValidatorsReadiness, ACTIVATION_HEIGHT},
    state::types::GenesisValidator,
};

/// Stake percentage that is considered to indicate that the validators are ready
pub const READY_PERCENTAGE: u8 = 80;

/// Sends a transaction to the Nimiq PoW chain to report that we are ready
/// The transaction format is defined as follow:
///   Sender: Validator address
///   Recipient: Burn address
///   Value: 100 Lunas
///   Data: TBD
///
pub fn generate_ready_tx(validator: String) -> OutgoingTransaction {
    info!(
        validator_address = validator,
        "Generating ready transaction"
    );
    OutgoingTransaction {
        from: validator,
        to: Address::burn_address().to_user_friendly_address(),
        value: 1, //Lunas
        fee: 0,
    }
}

/// Checks if we have seen a ready transaction from a validator in the specified range
pub async fn get_ready_txns(
    client: &Client,
    validator: String,
    block_window: Range<u32>,
) -> Vec<TransactionDetails> {
    if let Ok(transactions) = client.get_transactions_by_address(&validator, 10).await {
        let filtered_txns: Vec<TransactionDetails> = transactions
            .into_iter()
            .filter(|txn| {
                // Here we filter by current epoch
                (txn.block_number > block_window.start)
                    && (txn.block_number < block_window.end)
                    && (txn.to_address == Address::burn_address().to_user_friendly_address())
                    && txn.value == 1
            })
            .collect();
        filtered_txns
    } else {
        Vec::new()
    }
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
            Err(Error::Rpc)
        }
    }
}

/// Checks if enough validators are ready.
/// If thats the case, the number of slots which are ready are returned.
/// The validators_allocation is a HashMap from Validator to number of slots owned by that validator.
pub async fn check_validators_ready(
    client: &Client,
    validators: Vec<GenesisValidator>,
) -> ValidatorsReadiness {
    // First calculate the total amount of stake
    let total_stake: Coin = validators.iter().map(|validator| validator.balance).sum();

    log::debug!(registered_stake = %total_stake);

    let mut ready_validators = Vec::new();

    log::info!("Starting to collect transactions from validators...");

    // Now we need to collect all the transactions for each validator
    for validator in validators {
        let address = validator
            .validator
            .validator_address
            .to_user_friendly_address();
        if let Ok(transactions) = client.get_transactions_by_address(&address, 10).await {
            info!(
                num_transactions = transactions.len(),
                from_address = address,
                "Transactions found for validator"
            );
            // We only keep the ones past the activation window that met the activation criteria
            let filtered_txns: Vec<TransactionDetails> = transactions
                .into_iter()
                .filter(|txn| {
                    // Here we filter by the readiness criteria, TBD
                    (txn.block_number > ACTIVATION_HEIGHT)
                        && (txn.to_address == Address::burn_address().to_user_friendly_address())
                        && txn.value == 1
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
        ready_stake += ready_validator.balance;

        info!(
            address = ready_validator
                .validator
                .validator_address
                .to_user_friendly_address(),
            stake = %ready_validator.balance,
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
