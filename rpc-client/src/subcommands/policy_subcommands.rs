use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_rpc_interface::policy::PolicyInterface;

use super::accounts_subcommands::HandleSubcommand;
use crate::Client;

#[derive(Debug, Parser)]
pub enum PolicyCommand {
    /// Returns a bundle of policy constants.
    PolicyConstants {},

    /// Returns the epoch number at a given block number (height).
    EpochAt {
        /// The block number to fetch its epoch from.
        block_number: u32,
    },

    /// Returns the epoch index at a given block number. The epoch index is the number of a block relative
    /// to the epoch it is in. For example, the first block of any epoch always has an epoch index of 0.
    EpochIndexAt {
        /// The block number to fetch its epoch index from.
        block_number: u32,
    },

    /// Returns the batch number at a given `block_number` (height).
    BatchAt {
        /// The block number to fetch its batch number from.
        block_number: u32,
    },

    /// Returns the batch index at a given block number. The batch index is the number of a block relative
    /// to the batch it is in. For example, the first block of any batch always has an batch index of 0.
    BatchIndexAt {
        /// The block number to fetch its batch index from.
        block_number: u32,
    },

    /// Returns the number (height) of the next election macro block after a given block number (height).
    ElectionBlockAfter {
        /// The block number.
        block_number: u32,
    },

    /// Returns the number block (height) of the preceding election macro block before a given block number (height).
    /// If the given block number is an election macro block, it returns the election macro block before it.
    ElectionBlockBefore {
        /// The block number.
        block_number: u32,
    },

    /// Returns the block number (height) of the last election macro block at a given block number (height).
    /// If the given block number is an election macro block, then it returns that block number.
    LastElectionBlock {
        /// The block number.
        block_number: u32,
    },

    /// Returns a boolean expressing if the block at a given block number (height) is an election macro block.
    IsElectionBlockAt {
        /// The block number to query if its election block.
        block_number: u32,
    },

    /// Returns the block number (height) of the next macro block after a given block number (height).
    MacroBlockAfter {
        /// The block number.
        block_number: u32,
    },

    /// Returns the block number (height) of the preceding macro block before a given block number (height).
    /// If the given block number is a macro block, it returns the macro block before it.
    MacroBlockBefore {
        /// The block number.
        block_number: u32,
    },

    /// Returns block the number (height) of the last macro block at a given block number (height).
    /// If the given block number is a macro block, then it returns that block number.
    LastMacroBlock {
        /// The block number.
        block_number: u32,
    },

    /// Returns a boolean expressing if the block at a given block number (height) is a macro block.
    IsMacroBlockAt {
        /// The block number to query if macro block.
        block_number: u32,
    },

    /// Returns a boolean expressing if the block at a given block number (height) is a micro block.
    IsMicroBlockAt {
        /// The block number to query if micro block.
        block_number: u32,
    },

    /// Returns the block number of the first block of the given epoch (which is always a micro block).
    FirstBlockOf {
        /// The epoch number to get its first block number from.
        epoch: u32,
    },

    /// Returns the block number of the first block of the given batch (which is always a micro block).
    FirstBlockOfBatch {
        /// The batch number to get its first block number from.
        batch: u32,
    },

    /// Returns the block number of the election macro block of the given epoch (which is always the last block).
    ElectionBlockOf {
        /// The epoch number.
        epoch: u32,
    },

    /// Returns the block number of the macro block (checkpoint or election) of the given batch (which
    /// is always the last block).
    MacroBlockOf {
        /// The batch number.
        batch: u32,
    },

    /// Returns a boolean expressing if the batch at a given block number (height) is the first batch
    /// of the epoch.
    FirstBatchOfEpoch {
        /// The block number.
        block_number: u32,
    },

    /// Returns the supply at a given time (as Unix time) in Lunas (1 NIM = 100,000 Lunas). It is
    /// calculated using the following formula:
    /// Supply (t) = Genesis_supply + Initial_supply_velocity / Supply_decay * (1 - e^(- Supply_decay * t))
    /// Where e is the exponential function, t is the time in milliseconds since the genesis block and
    /// 'genesis_supply' is the supply at the genesis of the Nimiq 2.0 chain.
    SupplyAt {
        /// The supply at genesis.
        genesis_supply: u64,

        /// The time of genesis.
        genesis_time: u64,

        /// The current time.
        current_time: u64,
    },
}

#[async_trait]
impl HandleSubcommand for PolicyCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<Client, Error> {
        match self {
            PolicyCommand::PolicyConstants {} => {
                println!("{:#?}", client.policy.get_policy_constants().await?);
            }
            PolicyCommand::EpochAt { block_number } => {
                println!("{:#?}", client.policy.get_epoch_at(block_number).await?);
            }
            PolicyCommand::EpochIndexAt { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_epoch_index_at(block_number).await?
                );
            }
            PolicyCommand::BatchAt { block_number } => {
                println!("{:#?}", client.policy.get_batch_at(block_number).await?);
            }
            PolicyCommand::BatchIndexAt { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_batch_index_at(block_number).await?
                );
            }
            PolicyCommand::ElectionBlockAfter { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_election_block_after(block_number).await?
                );
            }
            PolicyCommand::ElectionBlockBefore { block_number } => {
                println!(
                    "{:#?}",
                    client
                        .policy
                        .get_election_block_before(block_number)
                        .await?
                );
            }
            PolicyCommand::LastElectionBlock { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_last_election_block(block_number).await?
                );
            }
            PolicyCommand::IsElectionBlockAt { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.is_election_block_at(block_number).await?
                );
            }
            PolicyCommand::MacroBlockAfter { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_macro_block_after(block_number).await?
                );
            }
            PolicyCommand::MacroBlockBefore { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_macro_block_before(block_number).await?
                );
            }
            PolicyCommand::LastMacroBlock { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_last_macro_block(block_number).await?
                );
            }
            PolicyCommand::IsMacroBlockAt { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.is_macro_block_at(block_number).await?
                );
            }
            PolicyCommand::IsMicroBlockAt { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.is_micro_block_at(block_number).await?
                );
            }
            PolicyCommand::FirstBlockOf { epoch } => {
                println!("{:#?}", client.policy.get_first_block_of(epoch).await?);
            }
            PolicyCommand::FirstBlockOfBatch { batch } => {
                println!(
                    "{:#?}",
                    client.policy.get_first_block_of_batch(batch).await?
                );
            }
            PolicyCommand::ElectionBlockOf { epoch } => {
                println!("{:#?}", client.policy.get_election_block_of(epoch).await?);
            }
            PolicyCommand::MacroBlockOf { batch } => {
                println!("{:#?}", client.policy.get_macro_block_of(batch).await?);
            }
            PolicyCommand::FirstBatchOfEpoch { block_number } => {
                println!(
                    "{:#?}",
                    client.policy.get_first_batch_of_epoch(block_number).await?
                );
            }
            PolicyCommand::SupplyAt {
                genesis_supply,
                genesis_time,
                current_time,
            } => {
                println!(
                    "{:#?}",
                    client
                        .policy
                        .get_supply_at(genesis_supply, genesis_time, current_time)
                        .await?
                );
            }
        }
        Ok(client)
    }
}
