use anyhow::Error;
use async_trait::async_trait;
use clap::ArgGroup;
use clap::Parser;
use futures::StreamExt;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_rpc_interface::blockchain::BlockchainInterface;
use nimiq_rpc_interface::types::{BlockNumberOrHash, LogType};

use crate::Client;

use super::accounts_subcommands::HandleSubcommand;

#[derive(Debug, Parser)]
pub enum BlockchainCommand {
    /// Returns the block number for the current head.
    BlockNumber {},

    /// Returns the batch number for the current head.
    BatchNumber {},

    /// Returns the epoch number for the current head.
    EpochNumber {},

    /// Query a block from the blockchain.
    Block {
        /// Either a block hash or number. If omitted, the last block is queried.
        hash_or_number: Option<BlockNumberOrHash>,

        /// Include transactions
        #[clap(short = 't')]
        include_transactions: bool,
    },

    /// Query a transaction from the blockchain.
    Transaction {
        /// The transation hash.
        hash: Blake2bHash,
    },

    /// Query for all transactions present within a block or batch.
    /// Block or batch number arguments are mutually exclusive, only exactly one of them can be provided.
    #[clap(group(
    ArgGroup::new("block_or_batch")
    .required(true)
    .args(&["block-number", "batch-number"]),
    ))]
    Transactions {
        /// The block number to fetch all its transactions.
        #[clap(conflicts_with = "batch-number", long)]
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
    .args(&["block-number", "batch-number"]),
    ))]
    Inherents {
        /// The block number to fetch all its inherents.
        #[clap(conflicts_with = "batch-number", long)]
        block_number: Option<u32>,

        /// The batch number to fetch all its inherents.
        #[clap(long)]
        batch_number: Option<u32>,
    },

    /// Returns the latests transactions or their hashes for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of transactions/hashes to
    /// fetch, it defaults to 500.
    TransactionsByAddress {
        /// The address to query by.
        address: Address,

        /// Max number of transactions to fetch. If absent it defaults to 500.
        #[clap(long)]
        max: Option<u16>,

        /// If set true only the hash of the transactions will be fetched. Otherwise the full transactions will be retrieved.
        #[clap(short = 't')]
        just_hash: bool,
    },

    /// Returns the information for the slot owner at the given block height and view number. The
    /// view number is optional, it will default to getting the view number for the existing block
    /// at the given height. We only have this information available for the last 2 batches at most.
    SlotAt {
        /// The block height to retrieve the slots information.
        block_number: u32,

        /// The view number to retrieve at the block height specified.
        #[clap(short, long)]
        view_number: Option<u32>,
    },

    /// Returns information about the currently slashed slots or the previous batch. This includes slots that lost rewards
    /// and that were disabled.
    SlashedSlots {
        /// To retrieve the previously slashed slots instead.
        previous_slashed: bool,
    },

    /// Returns information about the currently parked validators.
    ParkedValidators {},

    /// Tries to fetch a validator information given its address. It has an option to include a map
    /// containing the addresses and stakes of all the stakers that are delegating to the validator.
    ValidatorByAddress {
        /// The address to query by.
        address: Address,

        /// Include the stakers of the validator.
        #[clap(short = 's')]
        include_stakers: Option<bool>,
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
    /// If no addresses or no logtypes are provided it fetches all logs.
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
    async fn handle_subcommand(self, mut client: Client) -> Result<(), Error> {
        match self {
            BlockchainCommand::Block {
                hash_or_number,
                include_transactions,
            } => {
                let block = match hash_or_number {
                    Some(BlockNumberOrHash::Hash(hash)) => {
                        client
                            .blockchain
                            .get_block_by_hash(hash, Some(include_transactions))
                            .await
                    }
                    Some(BlockNumberOrHash::Number(number)) => {
                        client
                            .blockchain
                            .get_block_by_number(number, Some(include_transactions))
                            .await
                    }
                    None => {
                        client
                            .blockchain
                            .get_latest_block(Some(include_transactions))
                            .await
                    }
                }?;
                println!("{:#?}", block)
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
                view_number,
            } => {
                println!(
                    "{:#?}",
                    client
                        .blockchain
                        .get_slot_at(block_number, view_number)
                        .await?
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
                just_hash,
            } => {
                if just_hash {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_transaction_hashes_by_address(address, max)
                            .await?
                    )
                } else {
                    println!(
                        "{:#?}",
                        client
                            .blockchain
                            .get_transactions_by_address(address, max)
                            .await?
                    )
                }
            }
            BlockchainCommand::SlashedSlots { previous_slashed } => {
                if previous_slashed {
                    println!(
                        "{:#?}",
                        client.blockchain.get_current_slashed_slots().await?
                    )
                } else {
                    println!(
                        "{:#?}",
                        client.blockchain.get_previous_slashed_slots().await?
                    )
                }
            }
            BlockchainCommand::ParkedValidators {} => {
                println!("{:#?}", client.blockchain.get_parked_validators().await?)
            }
            BlockchainCommand::ValidatorByAddress {
                address,
                include_stakers,
            } => println!(
                "{:#?}",
                client
                    .blockchain
                    .get_validator_by_address(address, include_stakers)
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
                        println!("{:#?}", block);
                    }
                } else {
                    let mut stream = client.blockchain.subscribe_for_head_block_hash().await?;

                    while let Some(block_hash) = stream.next().await {
                        println!("{}", block_hash);
                    }
                }
            }
            BlockchainCommand::FollowValidator { address } => {
                let mut stream = client
                    .blockchain
                    .subscribe_for_validator_election_by_address(address)
                    .await?;
                while let Some(validator) = stream.next().await {
                    println!("{:#?}", validator);
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
                    println!("{:#?}", blocklog);
                }
            }
        }
        Ok(())
    }
}
