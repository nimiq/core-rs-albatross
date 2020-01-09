use std::collections::HashSet;
use std::sync::Arc;
use std::iter::FromIterator;

use rpc_server::{Server as RpcServer, JsonRpcConfig};
use rpc_server::handlers::*;

use crate::client::Client;
use crate::config::config::RpcServerConfig;
use crate::error::Error;
use crate::config::consts::default_bind;


pub fn initialize_rpc_server(client: &Client, config: RpcServerConfig) -> Result<RpcServer, Error> {
    let ip = config.bind_to.unwrap_or_else(default_bind);
    info!("Initializing RPC server: {}:{}", ip, config.port);

    // Configure RPC server
    let (username, password) = if let Some(credentials) = config.credentials {
        (Some(credentials.username), Some(credentials.password))
    }
    else {
        warn!("No password set for RPC server!");
        (None, None)
    };

    let methods = config.allowed_methods
        .map(|methods| HashSet::from_iter(methods))
        .unwrap_or_default();

    let corsdomain = config.corsdomain.unwrap_or_default();

    let json_rpc_config = JsonRpcConfig {
        username,
        password,
        methods,
        allowip: (), // TODO: config.allow_ips,
        corsdomain,
    };

    // Initialize RPC server
    let rpc_server = RpcServer::new(ip, config.port, json_rpc_config)?;
    let handler = Arc::clone(&rpc_server.handler);

    // Install RPC modules
    #[cfg(feature="validator")] {
        if let Some(validator) = client.validator() {
            let block_production_handler = BlockProductionAlbatrossHandler::new(validator);
            handler.add_module(block_production_handler);
        }
    }

    let blockchain_handler = BlockchainAlbatrossHandler::new(client.blockchain());
    handler.add_module(blockchain_handler);

    let consensus_handler = ConsensusHandler::new(client.consensus());
    handler.add_module(consensus_handler);

    let network_handler = NetworkHandler::new(&client.consensus());
    handler.add_module(network_handler);

    let wallet_handler = WalletHandler::new(client.environment());
    let wallet_manager = Arc::clone(&wallet_handler.unlocked_wallets);
    handler.add_module(wallet_handler);

    let mempool_handler = MempoolAlbatrossHandler::new(client.mempool(), Some(wallet_manager));
    handler.add_module(mempool_handler);

    Ok(rpc_server)
}
