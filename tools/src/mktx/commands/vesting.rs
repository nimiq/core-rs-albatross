use clap::Subcommand;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{hex_secret_key_to_pair, CommandError, TransactionOrProof};

#[derive(Debug, Subcommand)]
/// Builds vesting-related transactions that create and redeem vesting contracts
pub enum VestingCommands {
    /// Creates a transaction to create a new vesting contract
    Create {
        /// Hex-encoded private key of the account creating the vesting contract
        secret_key: String,
        /// Amount to lock up in the vesting contract in Lunas
        value: Coin,
        /// NQ address that receives the vested funds according to the schedule
        owner: Address,
        /// Start timestamp in milliseconds when vesting begins
        start_time: u64,
        /// Interval between releases expressed in milliseconds
        time_step: u64,
        /// Number of release steps in the schedule
        num_steps: u32,
    },
    /// Creates a transaction to redeem funds from a vesting contract
    Redeem {
        /// Hex-encoded private key of the vesting owner authorizing the redemption
        secret_key: String,
        /// Amount to redeem in Lunas
        value: Coin,
        /// NQ address of the vesting contract
        contract_address: Address,
        /// NQ address receiving the redeemed funds
        recipient: Address,
    },
}

pub fn get_tx(
    subcommand: VestingCommands,
    fee: Coin,
    validity_start_height: u32,
    network_id: NetworkId,
) -> Result<TransactionOrProof, CommandError> {
    match subcommand {
        VestingCommands::Create {
            secret_key,
            owner,
            start_time,
            time_step,
            num_steps,
            value,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_create_vesting(
                &hex_secret_key_to_pair(secret_key)?,
                owner,
                start_time,
                time_step,
                num_steps,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        VestingCommands::Redeem {
            secret_key,
            contract_address,
            recipient,
            value,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_redeem_vesting(
                &hex_secret_key_to_pair(secret_key)?,
                contract_address,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
    }
}
