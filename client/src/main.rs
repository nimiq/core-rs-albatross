extern crate toml;
#[macro_use]
extern crate serde_derive;
extern crate pretty_env_logger;
#[macro_use]
extern crate log;
extern crate futures;
extern crate tokio;
#[macro_use]
extern crate lazy_static;
#[cfg(debug_assertions)]
extern crate dotenv;

extern crate nimiq_database as database;
extern crate nimiq_network as network;
extern crate nimiq_consensus as consensus;
extern crate nimiq_primitives as primitives;
#[cfg(feature = "rpc-server")]
extern crate nimiq_rpc_server as rpc_server;
#[cfg(feature = "metrics-server")]
extern crate nimiq_metrics_server as metrics_server;
extern crate nimiq_lib as lib;


mod settings;


use std::error::Error;
use std::sync::{Arc, Mutex};
use std::net::IpAddr;
use std::str::FromStr;

use futures::{Future, future};

use network::network_config::NetworkConfig;
use primitives::networks::NetworkId;
use consensus::consensus::Consensus;
use network::network_config::ReverseProxyConfig;
use database::Environment;
use database::lmdb::{LmdbEnvironment, open};
use database::volatile::VolatileEnvironment;
#[cfg(feature = "rpc-server")]
use rpc_server::rpc_server;
#[cfg(feature = "metrics-server")]
use metrics_server::metrics_server;
use lib::client::{initialize, ClientInitializeFuture, ClientConnectFuture, ClientError};

use crate::settings::Settings;
use crate::settings as s;



lazy_static! {
    static ref ENV: Environment = LmdbEnvironment::new("./db/", 1024 * 1024 * 50, 10, open::Flags::empty()).unwrap();
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
    let consensus = Consensus::new(&ENV, network_id, network_config);
    info!("Blockchain state: height={}, head={}", consensus.blockchain.height(), consensus.blockchain.head_hash());

    // Additional futures we want to run.
    let mut other_futures: Vec<Box<dyn Future<Item=(), Error=()> + Send + Sync + 'static>> = Vec::new();

    // start RPC server if enabled
    #[cfg(feature = "rpc-server")] {
        let rpc_server = settings.rpc_server.map(|rpc_settings| {
            // TODO: Replace with parsing from config file
            let ip = IpAddr::from_str("127.0.0.1").unwrap();
            let port = rpc_settings.port.unwrap_or(s::DEFAULT_RPC_PORT);
            info!("Starting RPC server listening on port {}", port);
            other_futures.push(rpc_server(Arc::clone(&consensus), ip, port))
        });
    }
    // If the RPC server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "rpc-server"))] {
        if settings.rpc_server.is_some() {
            warn!("RPC server feature not enabled.");
        }
    }
    // start metrics server if enabled
    #[cfg(feature = "metrics-server")] {
        let metrics_server = settings.metrics_server.map(|metrics_settings| {
            // TODO: Replace with parsing from config file
            let ip = IpAddr::from_str("127.0.0.1").unwrap();
            let port = metrics_settings.port.unwrap_or(s::DEFAULT_METRICS_PORT);
            info!("Starting metrics server listening on port {}", port);
            other_futures.push(metrics_server(Arc::clone(&consensus), ip, port, metrics_settings.password))
        });
    }
    // If the metrics server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "metrics-server"))] {
        if settings.metrics_server.is_some() {
            warn!("Metrics server feature not enabled.");
        }
    }

    // Setup client future to initialize and connect
    // TODO: We might ass well pass an Arc of the whole consensus
    let client_future = initialize(Arc::clone(&consensus.network))
        .and_then(|client| client.connect());


    tokio::run(
        client_future // Run Nimiq client
            .map(|_| info!("Client finished")) // Map Result to None
            .map_err(|e| error!("Client failed: {}", e))
            .and_then( move |_| future::join_all(other_futures)) // Run other futures (e.g. RPC server)
            .map(|_| info!("Other futures finished"))
    );

    Ok(())
}
