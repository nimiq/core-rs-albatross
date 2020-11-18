use std::collections::HashSet;
use std::iter::FromIterator;

use nimiq_rpc_server::dispatchers::*;

use nimiq_jsonrpc_core::Credentials;
use nimiq_jsonrpc_server::{Config, Server as _Server, ModularDispatcher, AllowListDispatcher};

use crate::client::Client;
use crate::config::config::RpcServerConfig;
use crate::config::consts::default_bind;
use crate::error::Error;


pub type Server = _Server<AllowListDispatcher<ModularDispatcher>>;


pub fn initialize_rpc_server(client: &Client, config: RpcServerConfig) -> Result<Server, Error> {
    let ip = config.bind_to.unwrap_or_else(default_bind);
    info!("Initializing RPC server: {}:{}", ip, config.port);

    // Configure RPC server
    let basic_auth = config.credentials
        .map(|credentials| Credentials {
            username: credentials.username,
            password: credentials.password,
        });

    let allowed_methods = config
        .allowed_methods
        .map(HashSet::from_iter);

    // TODO: Pass this to the rpc server config
    let _corsdomain = config.corsdomain.unwrap_or_default();

    let mut dispatcher = ModularDispatcher::default();

    /*
    #[cfg(feature = "validator")]
    {
        if let Some(validator) = client.validator() {
            dispatcher.add(BlockProductionDispatcher::new(validator));
        }
    }
    */

    dispatcher.add(BlockchainDispatcher::new(client.blockchain()));
    //dispatcher.add(ConsensusDispatcher::new(client.consensus()));
    //dispatcher.add(NetworkDispatcher::new(client.consensus()));
    //let wallet_manager = Arc::clone(&wallet_handler.unlocked_wallets);
    //dispatcher.add(WalletDispatcher::new(wallet_manager));
    //dispatcher.add(MempoolHandler::new(client.mempool(), client.validator(), Some(wallet_manager)));

    Ok(Server::new(
        Config {
            bind_to: (
                config.bind_to.unwrap_or_else(default_bind),
                config.port,
            ).into(),
            enable_websocket: false,
            ip_whitelist: None,
            basic_auth,
        },
        AllowListDispatcher::new(dispatcher, allowed_methods),
    ))
}
