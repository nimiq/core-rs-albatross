use clap::Subcommand;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{
    hex_option_secret_key_to_option_pair, hex_secret_key_to_pair, CommandError, TransactionOrProof,
};

#[derive(Debug, Subcommand)]
/// Builds staking-related transactions that move a staker though its lifecycle: active, inactive, retired
pub enum StakeCommands {
    /// Creates a transaction to create a new staker
    Create {
        /// Hex-encoded private key of the account paying the stake and fee
        secret_key: String,
        /// Hex-encoded private key of the new staker; its derived address will be the staker address
        staker_secret_key: String,
        /// The amount of stake to lock up as initial active balance in Lunas
        value: Coin,
        /// Optional NQ address to delegate the stake to
        #[arg(long)]
        delegation: Option<Address>,
    },
    /// Creates a transaction to add more active stake to an existing staker; the coins come from a basic account
    Add {
        /// Hex-encoded private key of the basic account adding more stake
        secret_key: String,
        /// NQ address of the existing staker to add stake to
        staker_address: Address,
        /// Amount of stake to add in Lunas
        value: u64,
    },
    /// Creates a transaction to update the staker's settings; update the staker delegation and/or reactivate all inactive stake
    Update {
        /// Reactivate all inactive stake before applying the new delegation (if any); this moves from inactive to active and resets the release height
        #[arg(short, long)]
        reactivate_all_stake: bool,
        /// Hex-encoded staker private key authorizing the update
        staker_secret_key: String,
        /// Pay the transaction fee from the basic account belonging to this hex-encoded private key
        #[arg(long)]
        fee_key: Option<String>,
        /// The new validator NQ address to delegate to
        #[arg(long)]
        new_delegation: Option<Address>,
    },
    /// Sets the staker's active stake to a new value; can be used to both increase and decrease the active stake and restarts the lock-up timer
    Activate {
        /// Hex-encoded staker private key authorizing the balance rebalancing
        staker_secret_key: String,
        /// Target active stake balance in Lunas; the remaining stake (if any) becomes inactive
        new_active_balance: Coin,
        /// Optional hex-encoded private key of a basic account that should pay the fee; defaults to the staker
        #[arg(long)]
        fee_key: Option<String>,
    },
    /// Creates a transaction to retire part of the staker's stake; it moves already released inactive stake into the retired bucket so it can be withdrawn with the `remove` command
    Retire {
        /// Hex-encoded staker private key authorizing the stake retirement
        staker_secret_key: String,
        /// Amount of stake to retire in Lunas; must be <= the staker's inactive stake
        retire_stake: Coin,
        /// Optional hex-encoded private key of a basic account that should pay the fee; defaults to the staker
        #[arg(long)]
        fee_key: Option<String>,
    },
    /// Creates a transaction to remove the staker's entire retired stake; removes the staker once the total balance hits zero
    Remove {
        /// Hex-encoded private key of the staker authorizing the removal
        secret_key: String,
        /// NQ address of the basic account receiving the removed stake
        recipient: Address,
        /// Amount of stake to remove in Lunas; must equal the full retired balance
        value: Coin,
    },
}

pub fn get_tx(
    subcommand: StakeCommands,
    fee: Coin,
    validity_start_height: u32,
    network_id: NetworkId,
) -> Result<TransactionOrProof, CommandError> {
    match subcommand {
        StakeCommands::Create {
            secret_key,
            staker_secret_key,
            value,
            delegation,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_create_staker(
                &hex_secret_key_to_pair(secret_key)?,
                &hex_secret_key_to_pair(staker_secret_key)?,
                delegation,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        StakeCommands::Add {
            secret_key,
            staker_address,
            value,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_add_stake(
                &hex_secret_key_to_pair(secret_key)?,
                staker_address,
                Coin::from_u64_unchecked(value),
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        StakeCommands::Update {
            reactivate_all_stake,
            staker_secret_key,
            fee_key,
            new_delegation,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_update_staker(
                hex_option_secret_key_to_option_pair(fee_key)?.as_ref(),
                &hex_secret_key_to_pair(staker_secret_key)?,
                new_delegation,
                reactivate_all_stake,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        StakeCommands::Activate {
            staker_secret_key,
            new_active_balance,
            fee_key,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_set_active_stake(
                hex_option_secret_key_to_option_pair(fee_key)?.as_ref(),
                &hex_secret_key_to_pair(staker_secret_key)?,
                new_active_balance,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        StakeCommands::Retire {
            staker_secret_key,
            retire_stake,
            fee_key,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_retire_stake(
                hex_option_secret_key_to_option_pair(fee_key)?.as_ref(),
                &hex_secret_key_to_pair(staker_secret_key)?,
                retire_stake,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        StakeCommands::Remove {
            secret_key,
            recipient,
            value,
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_remove_stake(
                &hex_secret_key_to_pair(secret_key)?,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
    }
}
