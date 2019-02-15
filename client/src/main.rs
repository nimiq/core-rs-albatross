extern crate clap;
extern crate fern;
extern crate futures;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate nimiq_database as database;
extern crate nimiq_lib as lib;
#[cfg(feature = "metrics-server")]
extern crate nimiq_metrics_server as metrics_server;
extern crate nimiq_network as network;
extern crate nimiq_primitives as primitives;
extern crate nimiq_network_primitives as network_primitives;
#[cfg(feature = "rpc-server")]
extern crate nimiq_rpc_server as rpc_server;
#[cfg(feature = "deadlock-detection")]
extern crate parking_lot;
#[macro_use]
extern crate serde_derive;
extern crate tokio;
extern crate toml;


use std::io;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;

use failure::{Error, Fail};
use fern::log_file;
use futures::{Future, future};
use log::Level;

use database::lmdb::{LmdbEnvironment, open};
use lib::client::{ClientBuilder, Client};
#[cfg(feature = "metrics-server")]
use metrics_server::metrics_server;
use network::network_config::Seed;
use network_primitives::protocol::Protocol;
use primitives::networks::NetworkId;
#[cfg(feature = "rpc-server")]
use rpc_server::rpc_server;

use crate::cmdline::Options;
use crate::logging::{DEFAULT_LEVEL, NimiqDispatch, to_level};
use crate::logging::force_log_error_cause_chain;
use crate::settings as s;
use crate::settings::Settings;
use crate::static_env::ENV;

mod deadlock;
mod logging;
mod settings;
mod cmdline;
mod static_env;


/// Converts protocol from settings into 'normal' protocol
impl From<s::Protocol> for Protocol {
    fn from(protocol: s::Protocol) -> Protocol {
        match protocol {
            s::Protocol::Dumb => Protocol::Dumb,
            s::Protocol::Ws => Protocol::Ws,
            s::Protocol::Wss => Protocol::Wss,
            s::Protocol::Rtc => Protocol::Rtc,
        }
    }
}

/// Converts the network ID from settings into 'normal' network ID
impl From<s::Network> for NetworkId {
    fn from(network: s::Network) -> NetworkId {
        match network {
            s::Network::Main => NetworkId::Main,
            s::Network::Test => NetworkId::Test,
            s::Network::Dev=> NetworkId::Dev,
        }
    }
}


#[derive(Debug, Fail)]
pub enum ConfigError {
    #[fail(display = "No TLS identity file supplied")]
    NoTlsIdentityFile,
}

fn main() {
    #[cfg(feature = "deadlock-detection")]
    deadlock::deadlock_detection();

    log_panics::init();

    if let Err(e) = run() {
        force_log_error_cause_chain(e.as_fail(), Level::Error);
    }
}

fn run() -> Result<(), Error> {
    // parse command line arguments
    let cmdline = Options::parse()?;

    // load config file
    let config_path = cmdline.config_file.clone().unwrap_or("./config.toml".into());
    let settings = Settings::from_file(config_path)?;

    // Setup logging.
    let mut dispatch = fern::Dispatch::new()
        .pretty_logging(settings.log.timestamps)
        .level(DEFAULT_LEVEL)
        .level_for_nimiq(cmdline.log_level.as_ref().or(settings.log.level.as_ref()).map(|level| to_level(level)).unwrap_or(Ok(DEFAULT_LEVEL))?);
    for (module, level) in settings.log.tags.iter() {
        dispatch = dispatch.level_for(module.clone(), to_level(level)?);
    }
    // For now, we only log to stdout.
    dispatch = dispatch.chain(io::stdout());
    if let Some(ref filename) = settings.log.file {
        dispatch = dispatch.chain(log_file(filename)?);
    }
    dispatch.apply()?;

    debug!("Command-line options: {:#?}", cmdline);
    debug!("Settings: {:#?}", settings);

    // Start database and obtain a 'static reference to it.
    let default_database_settings = s::DatabaseSettings::default();
    let env = LmdbEnvironment::new(&settings.database.path, settings.database.size.unwrap_or(default_database_settings.size.unwrap()), settings.database.max_dbs.unwrap_or(default_database_settings.max_dbs.unwrap()), open::Flags::empty())?;
    // Initialize the static environment variable
    ENV.initialize(env);

    // Start building the client with network ID and environment
    let mut client_builder = ClientBuilder::new(Protocol::from(settings.network.protocol), ENV.get());

    // Map network ID from command-line or config to actual network ID
    client_builder.with_network_id(NetworkId::from(cmdline.network.unwrap_or(settings.consensus.network)));

    // add hostname and port to builder
    if let Some(hostname) = cmdline.hostname.or(settings.network.host) {
        client_builder.with_hostname(&hostname);
    }
    if let Some(port) = cmdline.port.or(settings.network.port) {
        client_builder.with_port(port);
    }

    if let Some(r) = settings.reverse_proxy {
        client_builder.with_reverse_proxy(
            r.port.unwrap_or(s::DEFAULT_REVERSE_PROXY_PORT),
            r.address.parse().expect("Could not parse reverse proxy address from config despite being enabled"),
            r.header,
            r.with_tls_termination
        );
    }

    // Add TLS configuration, if present
    // NOTE: Currently we only need to set TLS settings for Wss
    // TODO: Take ReverseProxyConfig and check if we're actually doing TLS
    if settings.network.protocol == s::Protocol::Wss {
        if let Some(tls_settings) = settings.tls {
            client_builder.with_tls_identity(&tls_settings.identity_file, &tls_settings.identity_password);
        }
    }

    // Parse additional seed nodes and add them
    client_builder.with_seeds(settings.network.seed_nodes.iter()
        .filter_map(|s| Seed::from_str(s).map_err(|e| {
            warn!("Invalid seed node: {}: {}", s, e);
            e
        }).ok()).collect::<Vec<Seed>>());

    // Setup client future to initialize and connect
    let client = client_builder.build_client()?;
    let consensus = client.consensus();

    // Additional futures we want to run.
    let mut other_futures: Vec<Box<dyn Future<Item=(), Error=()> + Send + Sync + 'static>> = Vec::new();

    // start RPC server if enabled
    // TODO: If no RPC server was configure, but on command line a port was given, generate RpcServer::default() and override port
    #[cfg(feature = "rpc-server")] {
        if let Some(rpc_settings) = settings.rpc_server {
            // TODO: Replace with parsing from config file
            let ip = IpAddr::from_str("127.0.0.1").unwrap();
            let port = rpc_settings.port.unwrap_or(s::DEFAULT_RPC_PORT);
            info!("Starting RPC server listening on port {}", port);
            other_futures.push(rpc_server(Arc::clone(&consensus), ip, port)?);
        }
    }
    // If the RPC server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "rpc-server"))] {
        if settings.rpc_server.is_some() {
            warn!("RPC server feature not enabled.");
        }
    }
    // start metrics server if enabled
    // TODO: Same as for RPC server
    #[cfg(feature = "metrics-server")] {
        if let Some(metrics_settings) = settings.metrics_server {
            // TODO: Replace with parsing from config file
            let ip = IpAddr::from_str("127.0.0.1").unwrap();
            let port = metrics_settings.port.unwrap_or(s::DEFAULT_METRICS_PORT);
            info!("Starting metrics server listening on port {}", port);
            other_futures.push(metrics_server(Arc::clone(&consensus), ip, port, metrics_settings.password)?);
        }
    }
    // If the metrics server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "metrics-server"))] {
        if settings.metrics_server.is_some() {
            warn!("Metrics server feature not enabled.");
        }
    }

    // Run client and other futures
    tokio::run(
        client.and_then(|c| c.connect()) // Run Nimiq client
            .map(|_| info!("Client finished")) // Map Result to None
            .map_err(|e| error!("Client failed: {}", e))
            .and_then( move |_| future::join_all(other_futures)) // Run other futures (e.g. RPC server)
            .map(|_| info!("Other futures finished"))
    );

    Ok(())
}
