use clap::Subcommand;

use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{hex_secret_key_to_pair, CommandError, TransactionOrProof};

#[derive(Debug, Subcommand)]
pub enum VestingCommands {
    Create {
        secret_key: String,
        value: Coin,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
    },
    Redeem {
        secret_key: String,
        value: Coin,
        contract_address: Address,
        recipient: Address,
    },
}

pub fn get_tx(subcommand: VestingCommands, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Result<TransactionOrProof, CommandError> {
    match subcommand {
        VestingCommands::Create {
            secret_key,
            owner,
            start_time,
            time_step,
            num_steps,
            value,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_create_vesting(
                &hex_secret_key_to_pair(secret_key)?,
                owner,
                start_time,
                time_step,
                num_steps,
                value,
                fee,
                validity_start_height,
                network_id,
            )?))
        }
        VestingCommands::Redeem {
            secret_key,
            contract_address,
            recipient,
            value,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_redeem_vesting(
                &hex_secret_key_to_pair(secret_key)?,
                contract_address,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id,
            )?))
        }
    }
}

