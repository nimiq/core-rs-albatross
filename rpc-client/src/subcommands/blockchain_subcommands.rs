use anyhow::Error;
use async_trait::async_trait;
use clap::{ArgGroup, Parser};
use futures::StreamExt;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_rpc_interface::{blockchain::BlockchainInterface, types::LogType};

use super::accounts_subcommands::HandleSubcommand;
use crate::Client;

#[derive(Debug, Parser)]
pub enum BlockchainCommand {
    /// Returns the block number for the current head.
    BlockNumber {},

    /// Returns the batch number for the current head.
    BatchNumber {},

    /// Returns the epoch number for the current head.
    EpochNumber {},

    /// Query a block from the blockchain either by block number or block hash.
    /// If omitted, the last block is queried.
    #[clap(group(
        ArgGroup::new("hash_or_number")
        .required(false)
        .args(&["block_hash", "block_number"]),
        ))]
    Block {
        /// The block hash of the desired block.
        #[clap(conflicts_with = "block_number", long)]
        block_hash: Option<Blake2bHash>,

        /// The block number of the desired block.
        #[clap(long)]
        block_number: Option<u32>,

        /// Whether to include the block body
        #[clap(short = 'b', long)]
        include_body: bool,
    },

    /// Query a transaction from the blockchain.
    Transaction {
        /// The transaction hash.
        hash: Blake2bHash,
    },

    /// Query for all transactions present within a block or batch.
    /// Block or batch number arguments are mutually exclusive, only exactly one of them can be provided.
    #[clap(group(
    ArgGroup::new("block_or_batch")
    .required(true)
    .args(&["block_number", "batch_number"]),
    ))]
    Transactions {
        /// The block number to fetch all its transactions.
        #[clap(conflicts_with = "batch_number", long)]
        block_number: Option<u32>,
        /// The batch number to fetch all its transactions.
        #[clap(long)]
        batch_number: Option<u32>,
    },

    /// Query for all inherents present within a block or batch.
    /// Block or batch number arguments are mutually exclusive, only exactly one of them can be provided.
    #[clap(group(
    ArgGroup::new("block_or_batch")
    .required(true)
    .args(&["block_number", "batch_number"]),
    ))]
    Inherents {
        /// The block number to fetch all its inherents.
        #[clap(conflicts_with = "batch_number", long)]
        block_number: Option<u32>,

        /// The batch number to fetch all its inherents.
        #[clap(long)]
        batch_number: Option<u32>,
    },

    /// Returns the latests transactions or their hashes for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of transactions/hashes to
    /// fetch, it defaults to 500. It has also an option to retrieve transactions before a given
    /// transaction hash. If this hash is not found or does not belong to this address, it will return an empty list.
    /// The transactions are returned in descending order, meaning the latest transaction is the first.
    TransactionsByAddress {
        /// The address to query by.
        address: Address,

        /// Max number of transactions to fetch. If absent it defaults to 500.
        #[clap(long)]
        max: Option<u16>,

        /// A transaction to start at.
        #[clap(long)]
        start_at: Option<Blake2bHash>,

        /// If set true only the hash of the transactions will be fetched. Otherwise the full transactions will be retrieved.
        #[clap(short = 'h')]
        just_hash: bool,
    },

    /// Returns the information for the slot owner at the given block height and offset. The
    /// offset is optional, it will default to the block number for micro blocks and to the round number for macro blocks.
    /// We only have this information available for the last 2 batches at most.
    SlotAt {
        /// The block height to retrieve the slots information.
        block_number: u32,

        /// The offset to retrieve at the block height specified.
        #[clap(short, long)]
        offset: Option<u32>,
    },

    /// Returns information about the currently penalized slots or the previous batch. This includes slots that lost rewards
    /// and that were disabled.
    PenalizedSlots {
        /// To retrieve the previously penalized slots instead.
        #[clap(short, long)]
        previous_penalized: bool,
    },

    /// Tries to fetch a validator information given its address.
    ValidatorByAddress {
        /// The address to query by.
        address: Address,
    },

    /// Tries to fetch all validators in the staking contract.
    /// IMPORTANT: This is a very expensive operation, iterating over all existing validators in the contract.
    Validators {},

    /// Tries to fetch all stakers of a given validator.
    /// IMPORTANT: This is a very expensive operation, iterating over all existing stakers in the contract.
    StakersByValidator {
        /// The validator address to query by.
        address: Address,
    },

    /// Tries to fetch a staker information given its address.
    Staker {
        /// The address to query by.
        address: Address,
    },

    /// Lists the current stakes from the staking contract.
    Stakes {},

    /// Follow the head of the blockchain.
    FollowHead {
        /// Show the full block instead of only the hash.
        #[clap(short)]
        block: bool,
    },

    /// Follow a validator state upon election blocks.
    FollowValidator {
        /// Validators address to subscribe to.
        address: Address,
    },

    /// Follow the logs associated with the specified addresses and of any of the log types given.
    /// If no addresses or no log types are provided it fetches all logs.
    FollowLogsOfAddressesAndTypes {
        /// List of all address to follow. If empty it does not filter by address.
        #[clap(short = 'a', long)]
        addresses: Vec<Address>,

        /// List of all log types to select. If empty it does not filter by log type.
        #[clap(short = 'l', long, value_enum)]
        log_types: Vec<LogType>,
    },
}

#[async_trait]
impl HandleSubcommand for BlockchainCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<Client, Error> {
        match self {
            BlockchainCommand::Block {
                block_hash,
                block_number,
                include_body,
            } => {
                let block = if let Some(block_hash) = block_hash {
                    client
                        .blockchain
                        .get_block_by_hash(block_hash, Some(include_body))
                        .await
                } else if let Some(block_number) = block_number {
                    client
                        .blockchain
                        .get_block_by_number(block_number, Some(include_body))
                        .await
                } else {
                    client.blockchain.get_latest_block(Some(include_body)).await
                }?;
                println!("{block:#?}")
            }
            BlockchainCommand::BlockNumber {} => {
                println!("{:#?}", client.blockchain.get_block_number().await?)
            }
            BlockchainCommand::BatchNumber {} => {
                println!("{:#?}", client.blockchain.get_batch_number().await?)
            }
            BlockchainCommand::EpochNumber {} => {
                println!("{:#?}", client.blockchain.get_epoch_number().await?)
            }
            BlockchainCommand::SlotAt {
                block_number,
                offset,
            } => {
                println!(
                    "{:#?}",
                    client.blockchain.get_slot_at(block_number, offset).await?
                )
            }
            BlockchainCommand::Transaction { hash } => {
                println!(
                    "{:#?}",
                    client.blockchain.get_transaction_by_hash(hash).await?
                )
            }
            BlockchainCommand::Transactions {
                block_number,
                batch_number,
            } => {
                if let Some(block_number) = block_number {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_transactions_by_block_number(block_number)
                            .await?
                    )
                } else {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_transactions_by_batch_number(batch_number.unwrap())
                            .await?
                    )
                }
            }
            BlockchainCommand::Inherents {
                block_number,
                batch_number,
            } => {
                if let Some(block_number) = block_number {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_inherents_by_block_number(block_number)
                            .await?
                    )
                } else {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_inherents_by_batch_number(batch_number.unwrap())
                            .await?
                    )
                }
            }

            BlockchainCommand::TransactionsByAddress {
                address,
                max,
                start_at,
                just_hash,
            } => {
                if just_hash {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_transaction_hashes_by_address(address, max, start_at)
                            .await?
                    )
                } else {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_transactions_by_address(address, max, start_at)
                            .await?
                    )
                }
            }
            BlockchainCommand::PenalizedSlots { previous_penalized } => {
                if previous_penalized {
                    println!(
                        "{:#?}",
                        client.blockchain.get_current_penalized_slots().await?
                    )
                } else {
                    println!(
                        "{:#?}",
                        client.blockchain.get_previous_penalized_slots().await?
                    )
                }
            }
            BlockchainCommand::ValidatorByAddress { address } => println!(
                "{:#?}",
                client.blockchain.get_validator_by_address(address).await?
            ),

            BlockchainCommand::Validators {} => {
                println!("{:#?}", client.blockchain.get_validators().await?)
            }

            BlockchainCommand::StakersByValidator { address } => println!(
                "{:#?}",
                client
                    .blockchain
                    .get_stakers_by_validator_address(address)
                    .await?
            ),
            BlockchainCommand::Staker { address } => {
                println!(
                    "{:#?}",
                    client.blockchain.get_staker_by_address(address).await?
                )
            }
            BlockchainCommand::Stakes {} => {
                println!("{:#?}", client.blockchain.get_active_validators().await?);
            }

            BlockchainCommand::FollowHead { block: show_block } => {
                if show_block {
                    let mut stream = client
                        .blockchain
                        .subscribe_for_head_block(Some(false))
                        .await?;

                    while let Some(block) = stream.next().await {
                        println!("{block:#?}");
                    }
                } else {
                    let mut stream = client.blockchain.subscribe_for_head_block_hash().await?;

                    while let Some(block_hash) = stream.next().await {
                        println!("{block_hash:#?}");
                    }
                }
            }
            BlockchainCommand::FollowValidator { address } => {
                let mut stream = client
                    .blockchain
                    .subscribe_for_validator_election_by_address(address)
                    .await?;
                while let Some(validator) = stream.next().await {
                    println!("{validator:#?}");
                }
            }
            BlockchainCommand::FollowLogsOfAddressesAndTypes {
                addresses,
                log_types,
            } => {
                let mut stream = client
                    .blockchain
                    .subscribe_for_logs_by_addresses_and_types(addresses, log_types)
                    .await?;

                while let Some(blocklog) = stream.next().await {
                    println!("{blocklog:#?}");
                }
            }
        }
        Ok(client)
    }
}
