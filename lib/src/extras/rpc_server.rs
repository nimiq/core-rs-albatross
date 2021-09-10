use std::{collections::HashSet, iter::FromIterator, sync::Arc};

use nimiq_rpc_server::dispatchers::*;

use nimiq_jsonrpc_core::Credentials;
use nimiq_jsonrpc_server::{AllowListDispatcher, Config, ModularDispatcher, Server as _Server};

use nimiq_wallet::WalletStore;

use crate::client::Client;
#[cfg(feature = "rpc-server")]
use crate::config::config::RpcServerConfig;
use crate::config::consts::default_bind;
use crate::error::Error;

pub type Server = _Server<AllowListDispatcher<ModularDispatcher>>;

#[cfg(feature = "rpc-server")]
pub fn initialize_rpc_server(
    client: &Client,
    config: RpcServerConfig,
    wallet_store: Arc<WalletStore>,
) -> Result<Server, Error> {
    let ip = config.bind_to.unwrap_or_else(default_bind);
    log::info!("Initializing RPC server: {}:{}", ip, config.port);

    // Configure RPC server
    let basic_auth = config.credentials.map(|credentials| Credentials {
        username: credentials.username,
        password: credentials.password,
    });

    let allowed_methods = config.allowed_methods.unwrap_or_default();
    let allowed_methods = if allowed_methods.is_empty() {
        None
    } else {
        Some(HashSet::from_iter(allowed_methods))
    };

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

    let wallet_dispatcher = WalletDispatcher::new(wallet_store);
    let unlocked_wallets = Arc::clone(&wallet_dispatcher.unlocked_wallets);

    dispatcher.add(BlockchainDispatcher::new(client.blockchain()));
    dispatcher.add(ConsensusDispatcher::new(
        client.consensus_proxy(),
        Some(unlocked_wallets),
    ));
    dispatcher.add(wallet_dispatcher);
    dispatcher.add(MempoolDispatcher::new(client.mempool()));
    dispatcher.add(NetworkDispatcher::new(client.network()));

    Ok(Server::new(
        Config {
            bind_to: (config.bind_to.unwrap_or_else(default_bind), config.port).into(),
            enable_websocket: false,
            ip_whitelist: None,
            basic_auth,
        },
        AllowListDispatcher::new(dispatcher, allowed_methods),
    ))
}
