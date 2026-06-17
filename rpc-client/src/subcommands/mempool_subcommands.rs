use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_hash::Blake2bHash;
use nimiq_rpc_client::MempoolInterface;

use super::accounts_subcommands::HandleSubcommand;
use crate::Client;

#[derive(Debug, Parser)]
pub enum MempoolCommand {
    /// Pushes the given serialized transaction to the local mempool with normal or high priority.
    PushTransaction {
        /// The raw transaction (in hex) to be pushed to the local mempool.
        raw_tx: String,

        /// To set this transaction as high priority.
        #[clap(short = 'p', long)]
        high_priority: bool,
    },

    /// Returns the hashes or the full transactions of the local mempool.
    MempoolContent {
        /// Includes the full transactions.
        #[clap(short = 't', long)]
        include_transactions: bool,
    },

    /// Returns information about the local mempool.
    MempoolInfo {},

    /// Returns the minimum fee per byte of the local mempool.
    MinFeePerByte {},

    /// Returns a transaction from the local mempool given its hash.
    MempoolTransaction {
        /// The hash of the transaction to fetch from the local mempool.
        hash: Blake2bHash,
    },
}

#[async_trait]
impl HandleSubcommand for MempoolCommand {
    async fn handle_subcommand(self, client: Client) -> Result<Client, Error> {
        match self {
            MempoolCommand::PushTransaction {
                raw_tx,
                high_priority,
            } => {
                if high_priority {
                    println!(
                        "{:#?}",
                        client
                            .mempool
                            .push_high_priority_transaction(raw_tx)
                            .await?
                    );
                } else {
                    println!("{:#?}", client.mempool.push_transaction(raw_tx).await?);
                }
            }
            MempoolCommand::MempoolContent {
                include_transactions,
            } => {
                println!(
                    "{:#?}",
                    client.mempool.mempool_content(include_transactions).await?
                );
            }
            MempoolCommand::MempoolInfo {} => {
                println!("{:#?}", client.mempool.mempool().await?);
            }
            MempoolCommand::MinFeePerByte {} => {
                println!("{:#?}", client.mempool.get_min_fee_per_byte().await?);
            }
            MempoolCommand::MempoolTransaction { hash } => {
                println!(
                    "{:#?}",
                    client.mempool.get_transaction_from_mempool(hash).await?
                );
            }
        }
        Ok(client)
    }
}
