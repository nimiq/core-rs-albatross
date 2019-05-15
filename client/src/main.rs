#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[cfg(feature = "deadlock-detection")]
extern crate parking_lot;
#[macro_use]
extern crate serde_derive;
#[cfg(feature = "human-panic")]
#[macro_use]
extern crate human_panic;

extern crate nimiq_database as database;
extern crate nimiq_lib as lib;
extern crate nimiq_mempool as mempool;
#[cfg(feature = "metrics-server")]
extern crate nimiq_metrics_server as metrics_server;
extern crate nimiq_network as network;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;
#[cfg(feature = "rpc-server")]
extern crate nimiq_rpc_server as rpc_server;
extern crate nimiq_keys as keys;
extern crate nimiq_utils as utils;
extern crate nimiq_consensus as consensus;
extern crate nimiq_bls as bls;


mod deadlock;
mod logging;
mod settings;
mod cmdline;
mod static_env;
mod serialization;
mod files;


use std::io;
use std::str::FromStr;
use std::sync::Arc;
use std::collections::HashSet;
use std::iter::FromIterator;
use std::env;
use std::path::PathBuf;

use failure::{Error, Fail};
use fern::log_file;
use futures::{Future, future};
use log::Level;

use database::lmdb::{LmdbEnvironment, open};
use lib::client::{Client, ClientBuilder, ClientInitializeFuture};
use mempool::MempoolConfig;
#[cfg(feature = "metrics-server")]
use metrics_server::metrics_server;
use network_primitives::protocol::Protocol;
use network_primitives::address::NetAddress;
use network::network_config::Seed;
use utils::key_store::KeyStore;
use primitives::networks::NetworkId;
#[cfg(feature = "rpc-server")]
use rpc_server::{rpc_server, Credentials, JsonRpcConfig};
use consensus::{ConsensusProtocol, AlbatrossConsensusProtocol, NimiqConsensusProtocol};

use crate::cmdline::Options;
use crate::logging::{DEFAULT_LEVEL, NimiqDispatch};
use crate::logging::force_log_error_cause_chain;
use crate::settings as s;
use crate::settings::Settings;
use crate::static_env::ENV;
use crate::serialization::SeedError;
use crate::files::LazyFileLocations;
use lib::block_producer::{BlockProducer, DummyBlockProducer};
use lib::block_producer::albatross::{ValidatorConfig, AlbatrossBlockProducer};
use lib::block_producer::mock::MockBlockProducer;
use bls::bls12_381::{SecretKey, PublicKey, KeyPair};
use beserial::Deserialize;


#[derive(Debug, Fail)]
pub enum ConfigError {
    #[fail(display = "Please configure a TLS identity file and the password in the `[tls]` section.")]
    NoTlsIdentityFile,
    #[fail(display = "Please configure a hostname in the `[network]` section.")]
    NoHostname,
    #[fail(display = "Invalid IP address.")]
    InvalidIpAddress,
    #[fail(display = "Username or password missing for RPC server.")]
    MissingRpcCredentials,
    #[fail(display = "The public key for a seed node is missing. Seed nodes without public_key are currently not implemented.")]
    MissingPublicKey,
    #[fail(display = "Config file not found")]
    MissingConfigFile
}

fn main() {
    #[cfg(feature = "deadlock-detection")]
    deadlock::deadlock_detection();

    // setup a human-readable panic message
    //#[cfg(feature = "human-panic")]
    //setup_panic!();

    if let Err(e) = run() {
        force_log_error_cause_chain(e.as_fail(), Level::Error);
    }
}

fn find_config_file(cmdline: &Options, files: &mut LazyFileLocations) -> Result<PathBuf, Error> {
    //  1. try from command line option
    if let Some(path) = &cmdline.config_file {
        return Ok(path.into());
    }

    //  2. try from environment variable
    if let Ok(path) = env::var("NIMIQ_CONFIG") {
        return Ok(path.into());
    }

    //  3. use default location: ~/.home/nimiq/client.toml or /etc/nimiq/client.toml
    // NOTE: Why is the panic in the error case not printed?
    Ok(files.config()?)
}

fn run_node<P, BP>(client_builder: ClientBuilder, settings: Settings, block_producer_config: BP::Config) -> Result<(), Error>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    let client: ClientInitializeFuture<P, BP> = client_builder.build_client::<P, BP>(block_producer_config)?;

    let consensus = client.consensus();

    info!("Peer address: {} - public key: {}", consensus.network.network_config.peer_address(), consensus.network.network_config.public_key().to_hex());

    // Additional futures we want to run.
    let mut other_futures: Vec<Box<dyn Future<Item=(), Error=()> + Send + Sync + 'static>> = Vec::new();

    // start RPC server if enabled
    #[cfg(feature = "rpc-server")] {
        if let Some(rpc_settings) = settings.rpc_server {
            // Unwrap is safe, since `NetAddress::from_str` only returns variants that can be turned
            // into a IpAddr
            let bind = rpc_settings.bind
                .unwrap_or_else(|| NetAddress::from_str("127.0.0.1").unwrap())
                .into_ip_address().unwrap();
            let port = rpc_settings.port.unwrap_or(s::DEFAULT_RPC_PORT);
            let credentials = match (rpc_settings.username, rpc_settings.password) {
                (Some(username), Some(password)) => Some(Credentials::new(&username, &password)),
                (None, None) => None,
                _ => Err(ConfigError::MissingRpcCredentials)?
            };
            if credentials.is_none() {
                warn!("Running RPC server without authentication! Consider setting a username and password.")
            }
            if !rpc_settings.corsdomain.is_empty() {
                warn!("Cross-Origin access is currently not implemented!");
            }
            if !rpc_settings.allowip.is_empty() {
                warn!("'allowip' for RPC server is currently not implemented!");
            }
            info!("Starting RPC server listening on port {}", port);
            /*other_futures.push(rpc_server(Arc::clone(&consensus), bind, port, JsonRpcConfig {
                credentials,
                methods: HashSet::from_iter(rpc_settings.methods),
                allowip: (), // TODO
                corsdomain: rpc_settings.corsdomain
            })?);*/
            //unimplemented!();
            warn!("No RPC server right now");
        }
    }
    // If the RPC server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "rpc-server"))] {
        if settings.rpc_server.is_some() {
            warn!("Client was compiled without RPC server");
        }
    }
    // start metrics server if enabled
    #[cfg(feature = "metrics-server")] {
        if let Some(metrics_settings) = settings.metrics_server {
            let bind = metrics_settings.bind
                .unwrap_or_else(|| NetAddress::from_str("127.0.0.1").unwrap())
                .into_ip_address().unwrap();
            let port = metrics_settings.port.unwrap_or(s::DEFAULT_METRICS_PORT);
            info!("Starting metrics server listening on port {}", port);
            //other_futures.push(metrics_server(Arc::clone(&consensus), bind, port, metrics_settings.password)?);
            //unimplemented!();
            warn!("No RPC server right now");
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
            .map(|_| info!("Client initialized")) // Map Result to None
            .map_err(|e| error!("Client initialization failed: {}", e))
            .and_then( move |_| future::join_all(other_futures)) // Run other futures (e.g. RPC server)
            .map(|_| info!("Other futures finished"))
    );

    Ok(())
}

fn run() -> Result<(), Error> {
    // parse command line arguments
    let cmdline = Options::parse()?;

    // default file locations
    let mut files = LazyFileLocations::new();

    // load config file
    let config_file = find_config_file(&cmdline, &mut files)?;
    if !config_file.exists() {
        eprintln!("Can't find config file at: {}", config_file.display());
        eprintln!("If you haven't configured the Nimiq client yet, do this by copying the client.example.toml to client.toml in the path above and editing it appropriately.");
        Err(ConfigError::MissingConfigFile)?;
    }
    let settings = Settings::from_file(&config_file)?;

    // Setup logging.
    let mut dispatch = fern::Dispatch::new()
        .pretty_logging(settings.log.timestamps)
        .level(DEFAULT_LEVEL)
        .level_for_nimiq(cmdline.log_level.as_ref()
            .map(|level| level.parse()).transpose()?
            .or(settings.log.level)
            .unwrap_or(DEFAULT_LEVEL));
    for (module, level) in settings.log.tags.iter().chain(cmdline.log_tags.iter()) {
        dispatch = dispatch.level_for(module.clone(), level.clone());
    }
    if let Some(ref filename) = settings.log.file {
        dispatch = dispatch.chain(log_file(filename)?);
    }
    else {
        dispatch = dispatch.chain(io::stderr());
    }
    dispatch.apply()?;
    #[cfg(not(feature = "human-panic"))]
    log_panics::init();

    info!("Loaded config file from: {}", config_file.display());
    debug!("Command-line options: {:#?}", cmdline);
    debug!("Settings: {:#?}", settings);

    // we only allow full nodes right now
    if settings.consensus.node_type != s::NodeType::Full {
        error!("Only full consensus is implemented right now.");
        unimplemented!();
    }

    // get network ID
    let network_id = NetworkId::from(cmdline.network.unwrap_or(settings.consensus.network));

    // Start database and obtain a 'static reference to it.
    let default_database_settings = s::DatabaseSettings::default();
    let env = LmdbEnvironment::new(&settings.database.path.clone()
        .unwrap_or_else(|| files.database(network_id).expect("Failed to find database").to_str().unwrap().to_string()),
                                   settings.database.size.unwrap_or_else(|| default_database_settings.size.unwrap()),
                                   settings.database.max_dbs.unwrap_or_else(|| default_database_settings.max_dbs.unwrap()),
                                   open::Flags::empty())?;
    // Initialize the static environment variable
    ENV.initialize(env);

    // open peer key store
    let peer_key_store = KeyStore::new(settings.peer_key_file.clone()
        .unwrap_or_else(|| files.peer_key()
            .expect("Failed to find peer key file")
            .to_str().unwrap().into()));

    // Start building the client with network ID and environment
    let mut client_builder = ClientBuilder::new(Protocol::from(settings.network.protocol), ENV.get(), peer_key_store);

    // Map network ID from command-line or config to actual network ID
    client_builder.with_network_id(network_id);

    // add hostname and port to builder
    if let Some(hostname) = cmdline.hostname.or_else(|| settings.network.host.clone()) {
        client_builder.with_hostname(&hostname);
    }
    else if settings.network.protocol == s::Protocol::Ws || settings.network.protocol == s::Protocol::Wss {
        Err(ConfigError::NoHostname)?
    }
    if let Some(port) = cmdline.port.or(settings.network.port) {
        client_builder.with_port(port);
    }

    // add reverse proxy settings to builder
    if let Some(ref r) = settings.reverse_proxy {
        client_builder.with_reverse_proxy(
            r.port.unwrap_or(s::DEFAULT_REVERSE_PROXY_PORT),
            r.address,
            r.header.clone(),
            r.with_tls_termination
        );
    }

    // Add mempool settings to filter
    if let Some(mempool_settings) = settings.mempool.clone() {
        client_builder.with_mempool_config(MempoolConfig::from(mempool_settings.clone()));
    }

    // Add TLS configuration, if present
    // NOTE: Currently we only need to set TLS settings for Wss
    if settings.network.protocol == s::Protocol::Wss {
        if let Some(tls_settings) = settings.network.tls.clone() {
            client_builder.with_tls_identity(&tls_settings.identity_file, &tls_settings.identity_password);
        }
        else {
            Err(ConfigError::NoTlsIdentityFile)?
        }
    }

    // Parse additional seed nodes and add them
    let seeds = settings.network.seed_nodes.iter()
        .map(|s| s::Seed::try_from(s.clone()))
        .collect::<Result<Vec<Seed>, SeedError>>()?;
    if seeds.iter().any(|s| match s {
        Seed::Peer(uri) => uri.public_key().is_none(),
        _ => false
    }) {
        Err(ConfigError::MissingPublicKey)?;
    }
    client_builder.with_seeds(seeds);

    // Setup client future to initialize and connect
    if network_id.is_albatross() {
        warn!("!!!!");
        warn!("!!!! Albatross node running");
        warn!("!!!!");

        let secret_key = SecretKey::deserialize_from_vec(&hex::decode("49ea68eb6b8afdf4ca4d4c0a0b295c76ca85225293693bc30e755476492b707f")
            .expect("Invalid hex encdoing"))
            .expect("Invalid secret key");
        let validator_config = ValidatorConfig {
            validator_key: KeyPair::from(secret_key)
        };

        //run_node::<AlbatrossConsensusProtocol, AlbatrossBlockProducer>(client_builder, settings, validator_config)?;
        run_node::<AlbatrossConsensusProtocol, MockBlockProducer>(client_builder, settings, ())?;
    }
    else {
        run_node::<NimiqConsensusProtocol, DummyBlockProducer>(client_builder, settings, ())?;
    }

    Ok(())
}
