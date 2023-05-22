use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;

use nimiq_rpc_interface::network::NetworkInterface;

use super::accounts_subcommands::HandleSubcommand;
use crate::Client;

#[derive(Debug, Parser)]
pub enum NetworkCommand {
    /// Returns the peer ID for our local peer.
    PeerId {},

    /// Returns the number of peers or lists all of them.
    Peers {
        /// To display only the number of peers.
        #[clap(short, long)]
        count: bool,
    },
}

#[async_trait]
impl HandleSubcommand for NetworkCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<Client, Error> {
        match self {
            NetworkCommand::PeerId {} => {
                println!("{:#?}", client.network.get_peer_id().await?);
            }
            NetworkCommand::Peers { count } => {
                if count {
                    println!("{:#?}", client.network.get_peer_count().await?);
                } else {
                    println!("{:#?}", client.network.get_peer_list().await?);
                }
            }
        }
        Ok(client)
    }
}
