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
use rand::rngs::OsRng;

use database::lmdb::{LmdbEnvironment, open};
use mempool::MempoolConfig;
use network_primitives::protocol::Protocol;
use network_primitives::address::NetAddress;
use network::network_config::Seed;
use utils::key_store::KeyStore;
use primitives::networks::NetworkId;
use consensus::{ConsensusProtocol, AlbatrossConsensusProtocol, NimiqConsensusProtocol};
use bls::bls12_381::KeyPair;
use network_primitives::services::ServiceFlags;
#[cfg(feature = "metrics-server")]
use metrics_server::{metrics_server, AlbatrossChainMetrics, NimiqChainMetrics, AbstractChainMetrics};
#[cfg(feature = "rpc-server")]
use rpc_server::{rpc_server, Credentials, JsonRpcConfig, AbstractRpcHandler, RpcHandler};

use lib::block_producer::{BlockProducer, DummyBlockProducer};
use lib::block_producer::albatross::{ValidatorConfig, AlbatrossBlockProducer};
use lib::block_producer::mock::MockBlockProducer;
use lib::error::ClientError;
use lib::client::{Client, ClientBuilder, ClientInitializeFuture};

use crate::cmdline::Options;
use crate::logging::{DEFAULT_LEVEL, NimiqDispatch};
use crate::logging::force_log_error_cause_chain;
use crate::settings as s;
use crate::settings::Settings;
use crate::static_env::ENV;
use crate::serialization::SeedError;
use crate::files::LazyFileLocations;


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

fn run_node<CC, BP>(client_builder: ClientBuilder, settings: Settings, block_producer_config: BP::Config) -> Result<(), Error>
    where CC: ClientConfiguration,
          BP: BlockProducer<CC::Protocol> + 'static,
{
    let client: ClientInitializeFuture<CC::Protocol, BP> = client_builder.build_client::<CC::Protocol, BP>(block_producer_config)?;

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
            other_futures.push(rpc_server::<CC::Protocol, CC::RpcHandler>(
                Arc::clone(&consensus), bind, port, JsonRpcConfig {
                    credentials,
                    methods: HashSet::from_iter(rpc_settings.methods),
                    allowip: (), // TODO
                    corsdomain: rpc_settings.corsdomain
                }
            )?);
        }
    }
    // If the RPC server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "rpc-server"))] {
        if settings.rpc_server.is_some() {
            warn!("Client was built without RPC server");
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
            other_futures.push(metrics_server::<CC::Protocol, CC::ChainMetrics>(
                Arc::clone(&consensus), bind, port, metrics_settings.password
            )?);
        }
    }
    // If the metrics server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "metrics-server"))] {
        if settings.metrics_server.is_some() {
            warn!("Client was built without Metrics server.");
        }
    }

    // Run client and other futures
    tokio::run(
        client
            .and_then(|c| c.connect()) // Run Nimiq client
            .and_then( move |c| future::join_all(other_futures)
                .map_err(|_| ClientError::OtherFailed)
                .and_then(|_| c)) // Run other futures (e.g. RPC server)
            .map_err(|e| error!("Client initialization failed: {}", e))
    );
    error!("Tokio exited");

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

        let validator_type = cmdline.validator_type.unwrap_or_else(|| settings.validator.ty.clone());
        match validator_type {
            s::ValidatorType::None => {
                info!("No validator");
                info!("Ignoring validator config");
                run_node::<AlbatrossConfiguration, DummyBlockProducer>(client_builder, settings, ())?;
            },
            s::ValidatorType::Mock => {
                info!("Mock validator");
                info!("Ignoring validator config");
                run_node::<AlbatrossConfiguration, MockBlockProducer>(client_builder, settings, ())?;
            },
            s::ValidatorType::Validator => {
                let validator_key = {
                    // Load validator key from key store, or create a new one, if key store doesn't exist
                    let key_store_file = settings.validator.key_file.clone()
                        .map(|s| Ok(PathBuf::from(s)))
                        .unwrap_or_else(|| files.validator_key())?;
                    let key_store = KeyStore::new(key_store_file.to_str().unwrap().to_string());
                    if !key_store_file.exists() {
                        info!("Generating validator key");
                        let key_pair = KeyPair::generate(&mut OsRng::new()?);
                        if let Err(ref err) = key_store.save_key(&key_pair) {
                            warn!("Failed to save key: {}", err);
                        }
                        key_pair
                    }
                    else {
                        key_store.load_key()?
                    }
                };

                client_builder.with_service_flags(ServiceFlags::VALIDATOR);

                let validator_config = ValidatorConfig {
                    validator_key,
                    block_delay: settings.validator.block_delay.unwrap_or(1000),
                };
                run_node::<AlbatrossConfiguration, AlbatrossBlockProducer>(client_builder, settings, validator_config)?;
            },
        };
    }
    else {
        run_node::<NimiqConfiguration, DummyBlockProducer>(client_builder, settings, ())?;
    }

    Ok(())
}

trait ClientConfiguration {
    type Protocol: ConsensusProtocol + 'static;
    type ChainMetrics: AbstractChainMetrics<Self::Protocol> + metrics_server::server::Metrics + 'static;
    type RpcHandler: AbstractRpcHandler<Self::Protocol> + 'static;
}

struct NimiqConfiguration();
impl ClientConfiguration for NimiqConfiguration {
    type Protocol = NimiqConsensusProtocol;
    type ChainMetrics = NimiqChainMetrics;
    type RpcHandler = RpcHandler<Self::Protocol>;
}

struct AlbatrossConfiguration();
impl ClientConfiguration for AlbatrossConfiguration {
    type Protocol = AlbatrossConsensusProtocol;
    type ChainMetrics = AlbatrossChainMetrics;
    type RpcHandler = RpcHandler<Self::Protocol>;
}
