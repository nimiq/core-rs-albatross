use clap::Subcommand;

use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{
    hex_secret_key_to_bls_pair,
    hex_secret_key_to_pair,
    hex_secret_key_to_public,
    hex_to_signal_data,
    CommandError,
    TransactionOrProof
};

#[derive(Debug, Subcommand)]
pub enum ValidatorCommands {
    Create {
        secret_key: String,
        secret_cold_key: String,
        secret_signing_key: String,
        secret_voting_key: String,
        reward_address: Address,
        signal_data: Option<String>,
    },
    Update {
        secret_key: String,
        secret_cold_key: String,
        /// Whether to overwrite signal data.
        /// If this flag is set but signal data is not provided, it will be deleted.
        #[arg(short, long)]
        overwrite_signal_data: bool,
        new_secret_signing_key: Option<String>,
        new_secret_voting_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
    },
    Deactivate {
        secret_key: String,
        validator_address: Address,
        secret_signing_key: String
    },
    Reactivate {
        secret_key: String,
        validator_address: Address,
        secret_signing_key: String,
    },
    Retire {
        secret_key: String,
        secret_cold_key: String,
    },
    Delete {
        recipient: Address,
        secret_cold_key: String,
        value: Coin,
    },
}

pub fn get_tx(subcommand: ValidatorCommands, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Result<TransactionOrProof, CommandError> {

    match subcommand {
        ValidatorCommands::Create {
            secret_key,
            secret_cold_key,
            secret_signing_key,
            secret_voting_key,
            reward_address,
            signal_data,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_create_validator(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(secret_cold_key)?,
                hex_secret_key_to_public(secret_signing_key)?,
                &hex_secret_key_to_bls_pair(secret_voting_key)?,
                reward_address,
                hex_to_signal_data(signal_data)?,
                fee,
                validity_start_height,
                network_id
            )?))
        }
        ValidatorCommands::Update {
            secret_key,
            secret_cold_key,
            new_secret_signing_key,
            new_secret_voting_key,
            new_reward_address,
            overwrite_signal_data: update_signal_data,
            new_signal_data,
        } => {
            let new_signing_key = match new_secret_signing_key {
                Some(secret_key) => Some(hex_secret_key_to_public(secret_key)?),
                None => None,
            };

            let new_voting_key_pair = match new_secret_voting_key {
                Some(secret_key) => Some(hex_secret_key_to_bls_pair(secret_key)?),
                None => None,
            };

            let new_signal_data = match update_signal_data {
                true => Some(hex_to_signal_data(new_signal_data)?),
                false => None,
            };

            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_update_validator(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(secret_cold_key)?,
                new_signing_key,
                new_voting_key_pair.as_ref(),
                new_reward_address,
                new_signal_data,
                fee,
                validity_start_height,
                network_id
            )))
        }
        ValidatorCommands::Deactivate {
            secret_key,
            validator_address,
            secret_signing_key,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_deactivate_validator(
                &hex_secret_key_to_pair(secret_key)?,
                validator_address,
                &hex_secret_key_to_pair(secret_signing_key)?,
                fee,
                validity_start_height,
                network_id
            )))
        }
        ValidatorCommands::Reactivate {
            secret_key,
            validator_address,
            secret_signing_key,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_reactivate_validator(
                &hex_secret_key_to_pair(secret_key)?,
                validator_address,
                &hex_secret_key_to_pair(secret_signing_key)?,
                fee,
                validity_start_height,
                network_id
            )))
        }
        ValidatorCommands::Retire {
            secret_key,
            secret_cold_key,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_retire_validator(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(secret_cold_key)?,
                fee,
                validity_start_height,
                network_id
            )))
        }
        ValidatorCommands::Delete {
            recipient,
            secret_cold_key,
            value,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_delete_validator(
                recipient,
                &hex_secret_key_to_pair(secret_cold_key)?,
                fee,
                value,
                validity_start_height,
                network_id
            )?))
        }
    }

}
