use clap::Subcommand;

use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{hex_option_secret_key_to_option_pair, hex_secret_key_to_pair, CommandError, TransactionOrProof};

#[derive(Debug, Subcommand)]
pub enum StakeCommands {
    /// Creates a transaction that creates a new staker with a given initial stake and delegation.
    Create {
        /// The key pair used to sign the outgoing transaction. The initial stake is sent from the basic account belonging to this key pair.
        secret_key: String,
        /// The key pair used to sign the incoming transaction. The staker address will be derived from this key pair.
        staker_secret_key: String,
        value: Coin,
        delegation: Option<Address>,
    },
    /// Creates a transaction to add stake from the address of a given `key_pair` to a specified `staker_address`.
    Add {
        secret_key: String,   
        staker_address: Address,
        value: u64,
    },
    /// Creates an update staker transaction for a given staker that changes the delegation.
    Update {
        /// Activate all inactive stake as part of the update.
        #[arg(short, long)]
        reactivate_all_stake: bool,
        /// Hex-encoded private key to the staker address that is being updated.
        staker_secret_key: String,
        /// Pay the transaction fee from the basic account belonging to this hex-encoded private key.
        fee_key: Option<String>,
        /// The new address to delegate to.
        new_delegation: Option<Address>,
    },
    /// Creates a set active stake transaction for a given staker.
    Activate {
        staker_secret_key: String,
        new_active_balance: Coin,
        secret_key: Option<String>,
    },
    Retire {
        staker_secret_key: String,
        retire_stake: Coin,
        secret_key: Option<String>,
    },
    Remove {
        secret_key: String,
        recipient: Address,
        value: Coin,
    },
}

pub fn get_tx(subcommand: StakeCommands, fee: Coin, validity_start_height: u32, network_id: NetworkId,) -> Result<TransactionOrProof, CommandError> {
    match subcommand {
        StakeCommands::Create {
            secret_key,
            staker_secret_key,
            value,
            delegation,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_create_staker(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(staker_secret_key)?,
                delegation,
                value,
                fee,
                validity_start_height,
                network_id,
            )?))
        }
        StakeCommands::Add {
            secret_key,
            staker_address,
            value,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_add_stake(
                &hex_secret_key_to_pair(secret_key)?,
                staker_address,
                Coin::from_u64_unchecked(value),
                fee,
                validity_start_height,
                network_id,
            )?))
        }
        StakeCommands::Update {
            reactivate_all_stake,
            staker_secret_key,
            fee_key,
            new_delegation,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_update_staker(
                hex_option_secret_key_to_option_pair(fee_key)?.as_ref(),
                &hex_secret_key_to_pair(staker_secret_key)?,
                new_delegation,
                reactivate_all_stake,
                fee,
                validity_start_height,
                network_id,
            )?))
        }
        StakeCommands::Activate {
            staker_secret_key,
            new_active_balance,
            secret_key,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_set_active_stake(
                hex_option_secret_key_to_option_pair(secret_key)?.as_ref(),
                &hex_secret_key_to_pair(staker_secret_key)?,
                new_active_balance,
                fee,
                validity_start_height,
                network_id
            )?))
        }
        StakeCommands::Retire {
            staker_secret_key,
            retire_stake,
            secret_key,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_retire_stake(
                hex_option_secret_key_to_option_pair(secret_key)?.as_ref(),
                &hex_secret_key_to_pair(staker_secret_key)?,
                retire_stake,
                fee,
                validity_start_height,
                network_id,
            )?))
        }
        StakeCommands::Remove {
            secret_key,
            recipient,
            value,
        } => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_remove_stake(
                &hex_secret_key_to_pair(secret_key)?,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id,
            )?))
        }
    }
}
