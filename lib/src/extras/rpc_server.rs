use std::{collections::HashSet, iter::FromIterator, sync::Arc};

use nimiq_jsonrpc_server::{
    AllowListDispatcher, Config, Cors, Credentials, ModularDispatcher, Server as _Server,
};
use nimiq_rpc_server::{dispatchers::*, eth_interface::*};
use nimiq_wallet::WalletStore;

#[cfg(feature = "rpc-server")]
use crate::config::config::RpcServerConfig;
use crate::{client::Client, config::consts::default_bind, error::Error};

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
    let basic_auth = config.credentials.map(|credentials| {
        Credentials::new_from_blake2b(credentials.username, credentials.password_hash.0 .0)
    });

    let allowed_methods = config.allowed_methods.unwrap_or_default();
    let allowed_methods = if allowed_methods.is_empty() {
        None
    } else {
        Some(HashSet::from_iter(allowed_methods))
    };

    let cors_domains = config.cors_domains.unwrap_or_default();
    let is_cors_wildcard = cors_domains.iter().any(|origin| origin.trim() == "*");
    let cors_config = if is_cors_wildcard {
        Cors::new().with_any_origin()
    } else {
        Cors::new().with_origins(cors_domains)
    };

    let mut dispatcher = ModularDispatcher::default();

    let wallet_dispatcher = WalletDispatcher::new(wallet_store);
    let unlocked_wallets = Arc::clone(&wallet_dispatcher.unlocked_wallets);

    dispatcher.add(BlockchainDispatcher::new(client.blockchain()));

    dispatcher.add(ConsensusDispatcher::new(
        client.consensus_proxy(),
        Some(unlocked_wallets),
    ));
    dispatcher.add(NetworkDispatcher::new(client.network()));
    if let Some(mempool) = client.mempool() {
        dispatcher.add(MempoolDispatcher::new(client.consensus_proxy(), mempool));
    }
    dispatcher.add(PolicyDispatcher {});
    if let Some(validator_state) = client.validator_state() {
        dispatcher.add(ValidatorDispatcher::new(
            validator_state,
            client.consensus_proxy(),
        ));
    }
    dispatcher.add(wallet_dispatcher);

    // Eth interface dispatchers
    dispatcher.add(GossipDispatcher::new(
        client.consensus_proxy(),
        client.blockchain(),
    ));

    dispatcher.add(HistoryDispatcher::new(client.blockchain()));

    dispatcher.add(StateDispatcher::new(client.blockchain()));

    Ok(Server::new(
        Config {
            bind_to: (config.bind_to.unwrap_or_else(default_bind), config.port).into(),
            // The RPC client connects over `/ws` and the subscription methods are only available
            // there, so websocket support must stay enabled
            enable_websocket: true,
            ip_whitelist: config.allow_ips.map(|ips| ips.into_iter().collect()),
            basic_auth,
            cors: Some(cors_config),
        },
        AllowListDispatcher::new(dispatcher, allowed_methods),
    ))
}
