use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_keys::Address;
use nimiq_rpc_client::OracleInterface;

use super::accounts_subcommands::HandleSubcommand;
use crate::Client;

#[derive(Debug, Parser)]
pub enum OracleCommand {
    /// Returns the current owner address of the oracle contract.
    GetOwner {
        /// The oracle contract address.
        contract_address: Address,
    },

    /// Returns the 0-based index of the last written hash.
    GetLatestIndex {
        /// The oracle contract address.
        contract_address: Address,
    },

    /// Returns the 0-based earliest index still retained in the ring buffer.
    GetEarliestIndex {
        /// The oracle contract address.
        contract_address: Address,
    },

    /// Returns the size of the sliding window (ring buffer capacity).
    GetWindowSize {
        /// The oracle contract address.
        contract_address: Address,
    },

    /// Returns the latest hash-chain head.
    GetLatestData {
        /// The oracle contract address.
        contract_address: Address,
    },

    /// Returns the data for a given 0-based index.
    /// Index must be in [earliest_index, latest_index] (inclusive).
    GetEntry {
        /// The oracle contract address.
        contract_address: Address,
        /// The 0-based index to retrieve.
        index: u64,
    },
}

#[async_trait]
impl HandleSubcommand for OracleCommand {
    async fn handle_subcommand(self, client: Client) -> Result<Client, Error> {
        match self {
            OracleCommand::GetOwner { contract_address } => {
                println!("{:#?}", client.oracle.get_owner(contract_address).await?);
            }
            OracleCommand::GetLatestIndex { contract_address } => {
                println!(
                    "{:#?}",
                    client.oracle.get_latest_index(contract_address).await?
                );
            }
            OracleCommand::GetEarliestIndex { contract_address } => {
                println!(
                    "{:#?}",
                    client.oracle.get_earliest_index(contract_address).await?
                );
            }
            OracleCommand::GetWindowSize { contract_address } => {
                println!(
                    "{:#?}",
                    client.oracle.get_window_size(contract_address).await?
                );
            }
            OracleCommand::GetLatestData { contract_address } => {
                println!(
                    "{:#?}",
                    client.oracle.get_latest_data(contract_address).await?
                );
            }
            OracleCommand::GetEntry {
                contract_address,
                index,
            } => {
                println!(
                    "{:#?}",
                    client.oracle.get_entry(contract_address, index).await?
                );
            }
        }
        Ok(client)
    }
}
