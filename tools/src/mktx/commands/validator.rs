use clap::Subcommand;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{
    hex_secret_key_to_bls_pair, hex_secret_key_to_pair, hex_secret_key_to_public,
    hex_to_signal_data, CommandError, TransactionOrProof,
};
use crate::commands::version_to_signal_data;

#[derive(Debug, Subcommand)]
/// Builds validator-related transactions that move a validator through its lifecycle: created, active, inactive, retired, deleted
pub enum ValidatorCommands {
    /// Creates a transaction to create a new validator by locking the fixed deposit and registering its keys
    Create {
        /// Hex-encoded private key of the account paying the deposit and fee
        secret_key: String,
        /// Hex-encoded private key of the validator's cold key; this key is used to create and delete the validator
        secret_cold_key: String,
        /// Hex-encoded private key of the validator's signing key; this key is used to sign the blocks and signal transactions
        secret_signing_key: String,
        /// Hex-encoded private key of the validator's voting key; this key is used to sign the votes for macro/skip blocks
        secret_voting_key: String,
        /// NQ address that receives the block rewards
        reward_address: Address,
        /// Optional hex-encoded signal data for validator coordination/upgrade signaling
        #[arg(long)]
        signal_data: Option<String>,
    },
    /// Creates a transaction to update the validator's settings; update the validator's keys, reward address, and/or signal data
    Update {
        /// Hex-encoded private key of the account paying the fee
        secret_key: String,
        /// Hex-encoded private key of the validator's cold key; this key proves ownership of the validator
        secret_cold_key: String,
        /// Optional replacement of the validator's hex-encoded signing key
        #[arg(long)]
        new_secret_signing_key: Option<String>,
        /// Optional replacement of the validator's hex-encoded voting key
        #[arg(long)]
        new_secret_voting_key: Option<String>,
        /// Optional replacement of the validator's reward address
        #[arg(long)]
        new_reward_address: Option<Address>,
        /// Optional hex-encoded replacement of the validator's signal data; omit both this
        /// and `--clear-signal-data` to leave the signal data unchanged
        #[arg(long, conflicts_with = "clear_signal_data")]
        new_signal_data: Option<String>,
        /// Clear the validator's signal data; mutually exclusive with `--new-signal-data`
        #[arg(long)]
        clear_signal_data: bool,
    },
    /// Creates a transaction that signals the validator's support for a new block version
    SignalUpgrade {
        /// Version this transaction will signal support for
        version: u16,
        /// Hex-encoded private key of the account paying the fee
        secret_key: String,
        /// Hex-encoded private key of the validator's cold key; this key proves ownership of the validator
        secret_cold_key: String,
    },
    /// Creates a transaction to deactivate an active validator; moves it to the inactive set
    Deactivate {
        /// Hex-encoded private key of the account paying the fee
        secret_key: String,
        /// NQ address of the validator to deactivate
        validator_address: Address,
        /// Hex-encoded private key of the validator's signing key to authorize the deactivation
        secret_signing_key: String,
    },
    /// Creates a transaction to reactivate an inactive validator; moves it back to the active set. Note: this can be automated in the validator's config file
    Reactivate {
        /// Hex-encoded private key of the account paying the fee
        secret_key: String,
        /// NQ address of the validator to reactivate
        validator_address: Address,
        /// Hex-encoded private key of the validator's signing key to authorize the reactivation
        secret_signing_key: String,
    },
    /// Creates a transaction to permanently retire a validator; forces deactivation if still active and this command is mandatory before `delete`
    Retire {
        /// Hex-encoded private key of the account paying the fee
        secret_key: String,
        /// Hex-encoded private key of the validator's cold key to authorize the retirement
        secret_cold_key: String,
    },
    /// Creates a transaction to delete a retired validator and withdraw its deposit (minus fee) to `recipient`; creates a tombstone if stakers remain
    Delete {
        /// NQ address that receives the validator's deposit minus fee
        recipient: Address,
        /// Hex-encoded private key of the validator's cold key signing the outgoing transaction
        secret_cold_key: String,
        /// Amount of deposit to withdraw in Lunas; must match the validator's deposit
        value: Coin,
    },
}

pub fn get_tx(
    subcommand: ValidatorCommands,
    fee: Coin,
    validity_start_height: u32,
    network_id: NetworkId,
) -> Result<TransactionOrProof, CommandError> {
    match subcommand {
        ValidatorCommands::Create {
            secret_key,
            secret_cold_key,
            secret_signing_key,
            secret_voting_key,
            reward_address,
            signal_data,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_create_validator(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(secret_cold_key)?,
                hex_secret_key_to_public(secret_signing_key)?,
                &hex_secret_key_to_bls_pair(secret_voting_key)?,
                reward_address,
                hex_to_signal_data(signal_data)?,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        ValidatorCommands::Update {
            secret_key,
            secret_cold_key,
            new_secret_signing_key,
            new_secret_voting_key,
            new_reward_address,
            new_signal_data,
            clear_signal_data,
        } => {
            let new_signing_key = match new_secret_signing_key {
                Some(secret_key) => Some(hex_secret_key_to_public(secret_key)?),
                None => None,
            };

            let new_voting_key_pair = match new_secret_voting_key {
                Some(secret_key) => Some(hex_secret_key_to_bls_pair(secret_key)?),
                None => None,
            };

            // TransactionBuilder's outer `Option` says whether to touch the signal data at all,
            // the inner one whether to set or clear it. `conflicts_with` on the flags rules
            // out the fourth combination, so the first arm can ignore `clear_signal_data`.
            let new_signal_data = match (new_signal_data, clear_signal_data) {
                (data @ Some(_), _) => Some(hex_to_signal_data(data)?),
                (None, true) => Some(None),
                (None, false) => None,
            };

            Ok(TransactionOrProof::Transaction(
                TransactionBuilder::new_update_validator(
                    &hex_secret_key_to_pair(secret_key)?,
                    &hex_secret_key_to_pair(secret_cold_key)?,
                    new_signing_key,
                    new_voting_key_pair.as_ref(),
                    new_reward_address,
                    new_signal_data,
                    fee,
                    validity_start_height,
                    network_id,
                ),
            ))
        }
        ValidatorCommands::SignalUpgrade {
            version,
            secret_key,
            secret_cold_key,
        } => {
            let new_signal_data = Some(Some(version_to_signal_data(version)?));

            Ok(TransactionOrProof::Transaction(
                TransactionBuilder::new_update_validator(
                    &hex_secret_key_to_pair(secret_key)?,
                    &hex_secret_key_to_pair(secret_cold_key)?,
                    None,
                    None,
                    None,
                    new_signal_data,
                    fee,
                    validity_start_height,
                    network_id,
                ),
            ))
        }
        ValidatorCommands::Deactivate {
            secret_key,
            validator_address,
            secret_signing_key,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_deactivate_validator(
                &hex_secret_key_to_pair(secret_key)?,
                validator_address,
                &hex_secret_key_to_pair(secret_signing_key)?,
                fee,
                validity_start_height,
                network_id,
            ),
        )),
        ValidatorCommands::Reactivate {
            secret_key,
            validator_address,
            secret_signing_key,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_reactivate_validator(
                &hex_secret_key_to_pair(secret_key)?,
                validator_address,
                &hex_secret_key_to_pair(secret_signing_key)?,
                fee,
                validity_start_height,
                network_id,
            ),
        )),
        ValidatorCommands::Retire {
            secret_key,
            secret_cold_key,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_retire_validator(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(secret_cold_key)?,
                fee,
                validity_start_height,
                network_id,
            ),
        )),
        ValidatorCommands::Delete {
            recipient,
            secret_cold_key,
            value,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_delete_validator(
                recipient,
                &hex_secret_key_to_pair(secret_cold_key)?,
                fee,
                value,
                validity_start_height,
                network_id,
            )?,
        )),
    }
}
