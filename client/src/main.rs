extern crate toml;
#[macro_use]
extern crate serde_derive;
extern crate pretty_env_logger;
#[macro_use]
extern crate log;
extern crate futures;
extern crate nimiq_database as database;
extern crate nimiq_network as network;
extern crate nimiq_consensus as consensus;
extern crate nimiq_primitives as primitives;
extern crate tokio;
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "rpc-server")]
extern crate nimiq_rpc_server as rpc_server;


#[cfg(debug_assertions)]
extern crate dotenv;

mod settings;
mod client_lib;

use std::error::Error;
use network::network_config::NetworkConfig;
use primitives::networks::NetworkId;
use consensus::consensus::Consensus;
use network::network_config::ReverseProxyConfig;
use database::Environment;
use database::lmdb::{LmdbEnvironment, open};
use database::volatile::VolatileEnvironment;
use futures::{Future, future};
use std::sync::Arc;

use crate::client_lib::{initialize, ClientInitializeFuture, ClientConnectFuture, ClientError};

use crate::settings::Settings;
use crate::settings as s;


#[cfg(feature = "rpc-server")]
use rpc_server::rpc_server;



lazy_static! {
    static ref env: Environment = LmdbEnvironment::new("./db/", 1024 * 1024 * 50, 10, open::Flags::empty()).unwrap();
}


// There is no really good way for a condition on the compilation environment (dev, staging, prod)
#[cfg(debug_assertions)]
fn dev_init() {
    dotenv::dotenv(); /*.expect("Couldn't load dotenv");*/
}

fn main() -> Result<(), Box<dyn Error>> {
    dev_init();

    pretty_env_logger::init();

    let argv: Vec<String> = std::env::args().collect();
    let path = argv.get(1).map(|p| p.as_str()).unwrap_or("config.toml");

    info!("Reading configuration: {}", path);
    let settings = Settings::from_file(path)?;

    debug!("Settings: {:#?}", settings);

    // TODO: If a wallet is specified, we should use it, shouldn't we?
    assert!(settings.wallet.is_none(), "Wallet settings are not implemented yet");

    // Start database
    // TODO: env must have lifetime 'static
    /*
    let env: Environment = (if settings.volatile {
        VolatileEnvironment::new(settings.database.max_dbs)?
    }
    else {
        LmdbEnvironment::new(&settings.database.path, settings.database.size, settings.database.max_dbs, open::Flags::empty())?
    });
    */

    // Start network
    let mut network_config = match settings.network.protocol {
        s::Protocol::Dumb => NetworkConfig::new_dumb_network_config(settings.network.user_agent),

        s::Protocol::Ws => NetworkConfig::new_ws_network_config(
            settings.network.host,
            settings.network.port.unwrap_or(s::DEFAULT_NETWORK_PORT),
            settings.reverse_proxy.map(|r| ReverseProxyConfig {
                port: r.port.unwrap_or(s::DEFAULT_REVERSE_PROXY_PORT),
                address: r.address,
                header: r.header,
            }), settings.network.user_agent),

        // TODO: What is a `identify_file`? Shouldn't this be a key and cert file?
        s::Protocol::Wss => NetworkConfig::new_wss_network_config(
            settings.network.host,
            settings.network.port.unwrap_or(s::DEFAULT_NETWORK_PORT),
            unimplemented!(), settings.network.user_agent),

        s::Protocol::Rtc => unimplemented!()
    };
    network_config.init_persistent();
    let network_id = match settings.consensus.network {
        s::Network::Main => NetworkId::Main,
        s::Network::Test => NetworkId::Test,
        s::Network::Dev => NetworkId::Dev,
        s::Network::Dummy => NetworkId::Dummy,
        s::Network::Bounty => NetworkId::Bounty
    };
    info!("Nimiq Core starting: network={:?}, peer_address={}", network_id, network_config.peer_address());

    // Start consensus
    let consensus = Consensus::new(&env, network_id, network_config);
    info!("Blockchain state: height={}, head={}", consensus.blockchain.height(), consensus.blockchain.head_hash());

    // start RPC server if enabled
    /*#[cfg(feature = "rpc-server")] {
        let rpc_server = settings.rpc_server.map(|rpc_settings| {
            let rpc_port = rpc_settings.port.unwrap_or(s::DEFAULT_RPC_PORT);
            info!("Starting RPC server listening on port {}", rpc_port);
            /* TODO: Fix `rpc_server` method
            rpc_server(Arc::clone(&consensus))
                .map_err(|e| {
                    error!("server error: {}", e);
                });
            */
            None
        });
    }*/
    #[cfg(not(feature = "rpc-server"))] {
        // If the RPC server is enabled, but the client is not compiled with it, inform the user
        if settings.rpc_server.is_some() {
            info!("RPC server feature not enabled.");
        }
    }

    // Setup client future to initialize and connect
    // TODO: We might ass well pass an Arc of the whole consensus
    let client_future = initialize(Arc::clone(&consensus.network))
        .and_then(|mut client| client.connect());

    // Additional futures we want to run.
    let mut other_futures: Vec<Box<dyn Future<Item=(), Error=()>>> = Vec::new();

    // if available add the rpc-server to the client future
    /*#[cfg(feature = "rpc-server")] {
        if let rpc_server = Some(rpc_server) {
            other_futures.push(rpc_server
                .map(|_| info!("RPC server finished"))
                .map_err(|e| error!("RPC server failed: {}", e)));
        }
    }*/

    tokio::run(
        client_future
            .map(|_| info!("Client finished")) // Map Result to None
            .map_err(|e| error!("Client failed: {}", e))
    );

    Ok(())
}
