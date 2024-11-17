use std::io;

use nimiq::{
    client::Client,
    config::{command_line::CommandLine, config::ClientConfig, config_file::ConfigFile},
    error::Error,
    extras::logging::{initialize_logging, log_error_cause_chain},
};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_genesis::NetworkInfo;
use nimiq_primitives::policy::Policy;
use nimiq_zkp_component::{proof_store::ProofStore, types::ZKPState};
use thiserror::Error;

#[derive(Debug, Error)]
enum NodeRecoveryError {
    #[error("Invalid input {0}")]
    InvalidInput(String),

    #[error("Reverting Blocks: {0}")]
    RevertingBlocksError(String),
}

async fn build_client() -> Result<(BlockchainProxy, Option<Box<dyn ProofStore>>), Error> {
    // Parse command line.
    let command_line = CommandLine::parse();
    log::trace!("Command line: {:#?}", command_line);

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    log::trace!("Config file: {:#?}", config_file);

    // Initialize logging with config values.
    initialize_logging(
        Some(&command_line),
        if command_line.prove {
            Some(&config_file.prover_log)
        } else {
            Some(&config_file.log)
        },
    )?;

    // Create config builder and apply command line and config file.
    // You usually want the command line to override config settings, so the order is important.
    let mut builder = ClientConfig::builder();
    builder.config_file(&config_file)?;
    builder.command_line(&command_line)?;

    // Finalize config.
    let config = builder.build()?;
    log::debug!("Final configuration: {:#?}", config);
    // Create client from config.
    let (blockchain_proxy, zkp_storage) = Client::from_config_dry(config).await?;

    Ok((blockchain_proxy, zkp_storage))
}

fn revert_blocks(
    blockchain: BlockchainProxy,
    zkp_db: Option<Box<dyn ProofStore>>,
    num_blocks: u32,
) -> Result<(), NodeRecoveryError> {
    let blockchain_proxy = blockchain.clone();

    let blockchain = match &blockchain {
        BlockchainProxy::Full(blockchain) => blockchain.upgradable_read(),
        BlockchainProxy::Light(_) => {
            return Err(NodeRecoveryError::RevertingBlocksError(
                "Reverting blocks is only available for history or full nodes.".to_string(),
            ))
        }
    };
    if !blockchain.state.accounts.is_complete(None) {
        return Err(NodeRecoveryError::RevertingBlocksError(
            "Accounts state is incomplete".to_string(),
        ));
    }

    let block_head = blockchain.head().block_number();
    if block_head <= num_blocks {
        return Err(NodeRecoveryError::InvalidInput(
            format!(
                "The number of bocks to revert is bigger than the current blockchain head height. num_blocks: {num_blocks}",
            )
            .to_string(),
        ));
    }
    if block_head.saturating_sub(num_blocks) < Policy::last_macro_block(block_head) {
        log::info!(
            "You are trying to revert past a macro block. Make sure you have a database backup before you proceed");
        log::info!("Type `y`/`yes` if you know what you are doing.");

        let mut data = String::new();
        io::stdin()
            .read_line(&mut data)
            .expect("Couldn't read user input.");

        if data.trim().to_lowercase() != "y" && data.trim().to_lowercase() != "yes" {
            log::info!("Ok, no action was performed. Hasta luego!");
            std::process::exit(0);
        }
    }

    if let Err(result) = Blockchain::revert_blocks_macro(blockchain, num_blocks) {
        return Err(NodeRecoveryError::RevertingBlocksError(
            format!("Couldn't revert the blocks. {:?}", result).to_string(),
        ));
    }

    // Remove the zkp if necessary.
    if block_head.saturating_sub(num_blocks) < Policy::last_election_block(block_head) {
        log::info!("We are reverting across a election block. The zkp state will also be reset.");
        let blockchain = match &blockchain_proxy {
            BlockchainProxy::Full(blockchain) => blockchain.upgradable_read(),
            BlockchainProxy::Light(_) => {
                return Err(NodeRecoveryError::RevertingBlocksError(
                    "Reverting blocks is only available for history or full nodes.".to_string(),
                ))
            }
        };
        if let Some(zkp_db) = zkp_db {
            let network_info = NetworkInfo::from_network_id(blockchain.network_id());
            let genesis_block = network_info.genesis_block().unwrap_macro();
            let a = ZKPState::with_genesis(&genesis_block).expect("Invalid genesis block");
            zkp_db.set_zkp(&a.into());
        }
    }

    log::error!(
        "Successfully reverted the blocks. New head: {}",
        blockchain_proxy.read().head()
    );

    Ok(())
}

#[tokio::main]
async fn main() {
    // Builds the client based on the arguments provided but does not spawn tasks.
    let result = build_client().await;
    if let Err(e) = result {
        log_error_cause_chain(&e);
        std::process::exit(1);
    }
    let (blockchain_proxy, zkp_db) = result.unwrap();

    println!(
        "Current head: {:?}",
        blockchain_proxy.read().head().block_number()
    );
    println!(
        "Last macro height {:?}",
        Policy::last_macro_block(blockchain_proxy.read().head().block_number())
    );
    println!(
        "Revert at least {:?}",
        blockchain_proxy.read().head().block_number()
            - Policy::last_macro_block(blockchain_proxy.read().head().block_number())
            + 1
    );

    // Ask user for the number of blocks to revert.
    println!("Enter the number of blocks to remove:");
    let mut data = String::new();
    io::stdin()
        .read_line(&mut data)
        .expect("Couldn't read user input.");
    let num_blocks: u32 = data.trim().parse().expect("Couldn't read user input.");

    // Revert the number of blocks specified
    let result = revert_blocks(blockchain_proxy, zkp_db, num_blocks);
    if let Err(e) = result {
        log_error_cause_chain(&e);
        std::process::exit(1);
    }

    std::process::exit(0);
}
