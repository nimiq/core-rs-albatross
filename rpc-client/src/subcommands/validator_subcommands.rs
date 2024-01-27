use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_keys::Address;
use nimiq_rpc_interface::{consensus::ConsensusInterface, validator::ValidatorInterface};

use super::{
    accounts_subcommands::HandleSubcommand,
    transactions_subcommands::{TxCommon, TxCommonWithValue},
};
use crate::Client;

#[derive(Debug, Parser)]
pub enum ValidatorCommand {
    /// Changes the automatic reactivation setting for the local validator.
    SetAutoReactivateValidator {
        /// The validator setting for automatic reactivation to be applied.
        #[clap(short, long)]
        automatic_reactivate: bool,
    },

    /// Returns the address of the local validator.
    ValidatorAddress {},

    /// Returns the signing key of the local validator.
    ValidatorSigningKey {},

    /// Returns the voting key of the local validator.
    ValidatorVotingKey {},

    /// Sends a `new_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit. The sender wallet must be unlocked
    /// prior to this command.
    /// The sender_wallet must be unlocked prior to this command.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. This becomes an issue when creating an update_validator transaction.
    /// Instead we use the following work-around. We define the empty String to be None. So, in
    /// this situation we have:
    /// "" = None
    /// "0x29a4b..." = Some(hash)
    CreateNewValidator {
        /// The fee will be paid from this address. This address must be already unlocked.
        sender_wallet: Address,

        /// The new validator address. This wallet must be already unlocked.
        validator_wallet: Address,

        /// The Schnorr signing key used by the validator.
        signing_secret_key: String,

        /// The BLS key used by the validator.
        voting_secret_key: String,

        /// The address to which the staking rewards are sent.
        reward_address: Address,

        /// The signal data showed by the validator.
        signal_data: String,

        #[clap(flatten)]
        tx_commons: TxCommon,
    },

    /// Sends a transaction to the network to update this validator. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the sender wallet must be unlocked prior to this command.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// null = No change in the signal data field.
    /// "" = Change the signal data field to None.
    /// "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
    UpdateValidator {
        /// The fee will be paid from this address. This wallet must be already unlocked.
        sender_wallet: Address,

        /// The new Schnorr signing key used by the validator.
        #[clap(long)]
        new_signing_secret_key: Option<String>,

        /// The new validator BLS key used by the validator.
        #[clap(long)]
        new_voting_secret_key: Option<String>,

        /// The new address to which the staking reward is sent.
        #[clap(long)]
        new_reward_address: Option<Address>,

        /// The new signal data showed by the validator.
        #[clap(short = 'd', long)]
        new_signal_data: Option<String>,

        #[clap(flatten)]
        tx_commons: TxCommon,
    },

    /// Sends a transaction to deactivate this validator. In order to avoid having the validator reactivated soon after
    /// this transaction takes effect, use the command set-auto-reactivate-validator to make sure the automatic reactivation
    /// configuration is turned off.
    /// The sender wallet must be unlocked prior to this command.
    DeactivateValidator {
        /// The fee will be paid from this address. This wallet must be already unlocked.
        sender_wallet: Address,

        #[clap(flatten)]
        tx_commons: TxCommon,
    },

    /// Sends a transaction to reactivate this validator. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    /// The sender wallet must be unlocked prior to this command.
    ReactivateValidator {
        /// The fee will be paid from this address. This wallet must be already unlocked.
        sender_wallet: Address,

        #[clap(flatten)]
        tx_commons: TxCommon,
    },

    /// Sends a transaction to delete this validator. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    DeleteValidator {
        /// The address to receive the balance of the validator.
        recipient_address: Address,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },
}

#[async_trait]
impl HandleSubcommand for ValidatorCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<Client, Error> {
        match self {
            ValidatorCommand::ValidatorAddress {} => {
                println!("{:#?}", client.validator.get_address().await?);
            }

            ValidatorCommand::ValidatorSigningKey {} => {
                println!("{:#?}", client.validator.get_signing_key().await?);
            }

            ValidatorCommand::ValidatorVotingKey {} => {
                println!("{:#?}", client.validator.get_voting_key().await?);
            }

            ValidatorCommand::SetAutoReactivateValidator {
                automatic_reactivate,
            } => {
                client
                    .validator
                    .set_automatic_reactivation(automatic_reactivate)
                    .await?;
                println!("Auto reactivate set to {automatic_reactivate}");
            }

            ValidatorCommand::CreateNewValidator {
                sender_wallet,
                validator_wallet,
                signing_secret_key,
                voting_secret_key,
                reward_address,
                signal_data,
                tx_commons,
            } => {
                if tx_commons.dry {
                    let tx = client
                        .consensus
                        .create_new_validator_transaction(
                            sender_wallet,
                            validator_wallet,
                            signing_secret_key,
                            voting_secret_key,
                            reward_address,
                            signal_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{tx:#?}");
                } else {
                    let txid = client
                        .consensus
                        .send_new_validator_transaction(
                            sender_wallet,
                            validator_wallet,
                            signing_secret_key,
                            voting_secret_key,
                            reward_address,
                            signal_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{txid:#?}");
                }
            }

            ValidatorCommand::UpdateValidator {
                sender_wallet,
                new_signing_secret_key,
                new_voting_secret_key,
                new_reward_address,
                new_signal_data,
                tx_commons,
            } => {
                let validator_address = client.validator.get_address().await?.data;
                if tx_commons.dry {
                    let tx = client
                        .consensus
                        .create_update_validator_transaction(
                            sender_wallet,
                            validator_address,
                            new_signing_secret_key,
                            new_voting_secret_key,
                            new_reward_address,
                            new_signal_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{tx:#?}");
                } else {
                    let txid = client
                        .consensus
                        .send_update_validator_transaction(
                            sender_wallet,
                            validator_address,
                            new_signing_secret_key,
                            new_voting_secret_key,
                            new_reward_address,
                            new_signal_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{txid:#?}");
                }
            }

            ValidatorCommand::DeactivateValidator {
                sender_wallet,
                tx_commons,
            } => {
                let validator_address = client.validator.get_address().await?.data;
                let key_data = client.validator.get_signing_key().await?.data;
                if tx_commons.dry {
                    let tx = client
                        .consensus
                        .create_deactivate_validator_transaction(
                            sender_wallet,
                            validator_address,
                            key_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{tx:#?}");
                } else {
                    let txid = client
                        .consensus
                        .send_deactivate_validator_transaction(
                            sender_wallet,
                            validator_address,
                            key_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{txid:#?}");
                }
            }

            ValidatorCommand::ReactivateValidator {
                sender_wallet,
                tx_commons,
            } => {
                let validator_address = client.validator.get_address().await?.data;
                let key_data = client.validator.get_signing_key().await?.data;
                if tx_commons.dry {
                    let tx = client
                        .consensus
                        .create_reactivate_validator_transaction(
                            sender_wallet,
                            validator_address,
                            key_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{tx:#?}");
                } else {
                    let txid = client
                        .consensus
                        .send_reactivate_validator_transaction(
                            sender_wallet,
                            validator_address,
                            key_data,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{txid:#?}");
                }
            }

            ValidatorCommand::DeleteValidator {
                recipient_address,
                tx_commons,
            } => {
                let validator_address = client.validator.get_address().await?.data;
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_delete_validator_transaction(
                            validator_address,
                            recipient_address,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.value,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{tx:#?}");
                } else {
                    let txid = client
                        .consensus
                        .send_delete_validator_transaction(
                            validator_address,
                            recipient_address,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.value,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{txid:#?}");
                }
            }
        }
        Ok(client)
    }
}
