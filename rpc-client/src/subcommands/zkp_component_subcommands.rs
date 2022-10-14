use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_rpc_interface::zkp_component::ZKPComponentInterface;

use crate::Client;

use super::accounts_subcommands::HandleSubcommand;

#[derive(Debug, Parser)]
pub enum ZKProverCommand {
    /// Returns the current zkp state.
    ZKPState {},
}

#[async_trait]
impl HandleSubcommand for ZKProverCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<(), Error> {
        match self {
            ZKProverCommand::ZKPState {} => {
                println!("{:?}", client.zkp_component.get_zkp_state().await?);
            }
        }
        Ok(())
    }
}
