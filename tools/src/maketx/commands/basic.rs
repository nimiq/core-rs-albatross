use clap::Args;

use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction_builder::TransactionBuilder;

use super::{hex_secret_key_to_pair, CommandError, TransactionOrProof};


#[derive(Debug, Args)]
pub struct AllArgs {
    /// The hex-encoded private key to access the sender account.
    hex_secret_key: String,
    /// The recipient address in user friendly format (NQXX XXXX XXXX ...).
    recipient: Address,
    /// The amount (in lunas) to transfer
    value: Coin,
    /// The hex-encoded data that will be stored in the transaction's data field.
    data: Option<String>,
}

pub fn get_tx(args: AllArgs, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Result<TransactionOrProof, CommandError> {
    let AllArgs {
        hex_secret_key,
        recipient,
        value,
        data,
    } = args;

    match data {
        None => {
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_basic(
                &hex_secret_key_to_pair(hex_secret_key)?,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id
            )?))
        },
        Some(data) => {
            let data = hex::decode(data)?;
            Ok(TransactionOrProof::Transaction(TransactionBuilder::new_basic_with_data(
                &hex_secret_key_to_pair(hex_secret_key)?,
                recipient,
                data,
                value,
                fee,
                validity_start_height,
                network_id
            )?))
        }
    }
}
