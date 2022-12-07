use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_rpc_interface::zkp_component::ZKPComponentInterface;

use crate::Client;

use super::accounts_subcommands::HandleSubcommand;

#[derive(Debug, Parser)]
pub enum ZKPComponentCommand {
    /// Returns the current zkp state.
    ZkpState {},
}

#[async_trait]
impl HandleSubcommand for ZKPComponentCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<(), Error> {
        match self {
            ZKPComponentCommand::ZkpState {} => {
                println!("{:?}", client.zkp_component.get_zkp_state().await?);
            }
        }
        Ok(())
    }
}
