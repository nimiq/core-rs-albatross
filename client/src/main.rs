#![feature(never_type)]

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
use parking_lot::RwLock;

use database::lmdb::{LmdbEnvironment, open};
use mempool::MempoolConfig;
use network_primitives::protocol::Protocol;
use network_primitives::address::NetAddress;
use network::network_config::Seed;
use utils::key_store::KeyStore;
use primitives::networks::NetworkId;
use consensus::{Consensus, ConsensusProtocol, AlbatrossConsensusProtocol, NimiqConsensusProtocol};
use bls::bls12_381::KeyPair;
use network_primitives::services::ServiceFlags;
#[cfg(feature = "metrics-server")]
use metrics_server::{metrics_server, AlbatrossChainMetrics, NimiqChainMetrics, AbstractChainMetrics};
#[cfg(feature = "rpc-server")]
use rpc_server::{
    rpc_server,
    Credentials, JsonRpcConfig, Handler as RpcHandler,
    handlers::blockchain_nimiq::BlockchainNimiqHandler,
    handlers::blockchain_albatross::BlockchainAlbatrossHandler,
    handlers::block_production_nimiq::BlockProductionNimiqHandler,
    handlers::block_production_albatross::BlockProductionAlbatrossHandler,
    handlers::consensus::ConsensusHandler,
    handlers::mempool::MempoolHandler,
    handlers::mempool_albatross::MempoolAlbatrossHandler,
    handlers::network::NetworkHandler,
    handlers::wallet::{WalletHandler, UnlockedWalletManager},
};

use lib::block_producer::{BlockProducer, DummyBlockProducer};
use lib::block_producer::albatross::{ValidatorConfig, AlbatrossBlockProducer};
use lib::error::ClientError;
use lib::client::{Client, ClientBuilder, ClientInitializeFuture};

use crate::cmdline::Options;
use crate::logging::{DEFAULT_LEVEL, NimiqDispatch};
use crate::logging::force_log_error_cause_chain;
use crate::settings as s;
use crate::settings::{Settings, RpcServerSettings};
use crate::serialization::SeedError;
use crate::files::LazyFileLocations;

type OtherFuture = Box<dyn Future<Item=(), Error=()> + Send + Sync + 'static>;

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

fn run_albatross_node(
    client_builder: ClientBuilder,
    settings: Settings,
    block_producer_config: <DummyBlockProducer as BlockProducer<AlbatrossConsensusProtocol>>::Config
) -> Result<!, Error> {
    let client: ClientInitializeFuture<AlbatrossConsensusProtocol, DummyBlockProducer> =
        client_builder.build_client(block_producer_config)?;
    let consensus = client.consensus();

    // Additional futures we want to run.
    let mut other_futures = build_other_futures::<AlbatrossConfiguration>(&settings, &consensus)?;

    // start RPC server if enabled
    #[cfg(feature = "rpc-server")] {
        if let Some((future, handler)) = build_rpc_server(settings.rpc_server)? {
            let unlocked_wallets = add_generic_rpc_modules(&handler, &consensus);

            let blockchain_handler = BlockchainAlbatrossHandler::new(Arc::clone(&consensus.blockchain));
            let mempool_handler = MempoolAlbatrossHandler::new(
                Arc::clone(&consensus.mempool),
                Some(unlocked_wallets),
            );

            handler.add_module(blockchain_handler);
            handler.add_module(mempool_handler);

            other_futures.push(future);
        }
    }

    run_client(client, other_futures)
}

fn run_albatross_validator_node(
    client_builder: ClientBuilder,
    settings: Settings,
    block_producer_config: <AlbatrossBlockProducer as BlockProducer<AlbatrossConsensusProtocol>>::Config
) -> Result<!, Error> {
    let client: ClientInitializeFuture<AlbatrossConsensusProtocol, AlbatrossBlockProducer> =
        client_builder.build_client(block_producer_config.clone())?;
    let consensus = client.consensus();

    // Additional futures we want to run.
    let mut other_futures = build_other_futures::<AlbatrossValidatorConfiguration>(&settings, &consensus)?;

    // start RPC server if enabled
    #[cfg(feature = "rpc-server")] {
        if let Some((future, mut handler)) = build_rpc_server(settings.rpc_server)? {
            let unlocked_wallets = add_generic_rpc_modules(&mut handler, &consensus);

            let public_key = block_producer_config.validator_key.public.compress();
            let proof_of_knowledge = block_producer_config.validator_key.sign(&public_key).compress();

            let blockchain_handler = BlockchainAlbatrossHandler::new(Arc::clone(&consensus.blockchain));
            let block_production_handler = BlockProductionAlbatrossHandler::new(public_key, proof_of_knowledge);
            let mempool_handler = MempoolAlbatrossHandler::new(
                Arc::clone(&consensus.mempool),
                Some(unlocked_wallets),
            );

            handler.add_module(blockchain_handler);
            handler.add_module(block_production_handler);
            handler.add_module(mempool_handler);

            other_futures.push(future);
        }
    }

    run_client(client, other_futures)
}

fn run_nimiq_node(
    client_builder: ClientBuilder,
    settings: Settings,
    block_producer_config: <DummyBlockProducer as BlockProducer<NimiqConsensusProtocol>>::Config
) -> Result<!, Error> {
    let client: ClientInitializeFuture<NimiqConsensusProtocol, DummyBlockProducer> =
        client_builder.build_client(block_producer_config)?;
    let consensus = client.consensus();

    // Additional futures we want to run.
    let mut other_futures = build_other_futures::<NimiqConfiguration>(&settings, &consensus)?;

    // start RPC server if enabled
    #[cfg(feature = "rpc-server")] {
        if let Some((future, mut handler)) = build_rpc_server(settings.rpc_server)? {
            let unlocked_wallets = add_generic_rpc_modules(&mut handler, &consensus);

            let blockchain_handler = BlockchainNimiqHandler::new(Arc::clone(&consensus.blockchain));
            let block_production_handler = BlockProductionNimiqHandler::new(
                Arc::clone(&consensus.blockchain),
                Arc::clone(&consensus.mempool),
            );
            let mempool_handler = MempoolHandler::<NimiqConsensusProtocol>::new(
                Arc::clone(&consensus.mempool),
                Some(unlocked_wallets),
            );

            handler.add_module(blockchain_handler);
            handler.add_module(block_production_handler);
            handler.add_module(mempool_handler);

            other_futures.push(future);
        }
    }

    run_client(client, other_futures)
}

fn run_client<P, BP>(client: ClientInitializeFuture<P, BP>, other_futures: Vec<OtherFuture>) -> Result<!, Error>
    where P: ConsensusProtocol + 'static,
          BP: BlockProducer<P> + 'static
{
    // Run client and other futures
    tokio::run(
        client
            .and_then(|c| c.connect()) // Run Nimiq client
            .and_then( move |c| future::join_all(other_futures)
                .map_err(|_| ClientError::OtherFailed)
                .and_then(|_| c)) // Run other futures (e.g. RPC server)
            .map_err(|e| error!("Client initialization failed: {}", e))
    );
    panic!("Tokio exited")
}

fn build_other_futures<CC>(settings: &Settings, consensus: &Arc<Consensus<CC::Protocol>>) -> Result<Vec<OtherFuture>, Error>
    where CC: ClientConfiguration
{
    let mut futures = Vec::<OtherFuture>::new();

    // If the RPC server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "rpc-server"))] {
        if settings.rpc_server.is_some() {
            warn!("Client was built without RPC server");
        }
    }

    // Start metrics server if enabled
    #[cfg(feature = "metrics-server")] {
        if let Some(ref metrics_settings) = settings.metrics_server {
            let bind = metrics_settings.bind
                .unwrap_or_else(|| NetAddress::from_str("127.0.0.1").unwrap())
                .into_ip_address().unwrap();
            let port = metrics_settings.port.unwrap_or(s::DEFAULT_METRICS_PORT);
            info!("Starting metrics server listening on port {}", port);
            futures.push(metrics_server::<CC::Protocol, CC::ChainMetrics>(
                Arc::clone(&consensus), bind, port, metrics_settings.password.clone()
            )?);
        }
    }
    // If the metrics server is enabled, but the client is not compiled with it, inform the user
    #[cfg(not(feature = "metrics-server"))] {
        if settings.metrics_server.is_some() {
            warn!("Client was built without Metrics server.");
        }
    }

    Ok(futures)
}

fn run() -> Result<!, Error> {
    // Parse command line arguments.
    let cmdline = Options::parse()?;

    // Default file locations.
    let mut files = LazyFileLocations::new();

    // Load config file.
    let config_file = find_config_file(&cmdline, &mut files)?;
    if !config_file.exists() {
        eprintln!("Can't find config file at: {}", config_file.display());
        eprintln!("If you haven't configured the Nimiq client yet, do this by copying the client.example.toml to client.toml in the path above and editing it appropriately.");
        return Err(ConfigError::MissingConfigFile.into());
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
    trace!("Command-line options: {:#?}", cmdline);
    trace!("Settings: {:#?}", settings);

    // We only allow full nodes right now.
    if settings.consensus.node_type != s::NodeType::Full {
        error!("Only full consensus is implemented right now.");
        unimplemented!();
    }

    // Get network ID.
    let network_id = NetworkId::from(cmdline.network.unwrap_or(settings.consensus.network));

    // Start database and obtain a 'static reference to it.
    let default_database_settings = s::DatabaseSettings::default();
    let env = LmdbEnvironment::new(&settings.database.path.clone()
        .unwrap_or_else(|| files.database(network_id).expect("Failed to find database").to_str().unwrap().to_string()),
            settings.database.size.unwrap_or_else(|| default_database_settings.size.unwrap()),
            settings.database.max_dbs.unwrap_or_else(|| default_database_settings.max_dbs.unwrap()),
            if settings.database.no_lmdb_sync.unwrap_or(false) { open::NOSYNC } else { open::NOMETASYNC })?;

    // Open peer key store.
    let peer_key_store = KeyStore::new(settings.peer_key_file.clone()
        .unwrap_or_else(|| files.peer_key()
            .expect("Failed to find peer key file")
            .to_str().unwrap().into()));

    // Start building the client with network ID and environment.
    let mut client_builder = ClientBuilder::new(Protocol::from(settings.network.protocol), env, peer_key_store);

    // Map network ID from command-line or config to actual network ID.
    client_builder.with_network_id(network_id);

    // Add hostname and port to builder.
    if let Some(hostname) = cmdline.hostname.or_else(|| settings.network.host.clone()) {
        client_builder.with_hostname(&hostname);
    }
    else if settings.network.protocol == s::Protocol::Ws || settings.network.protocol == s::Protocol::Wss {
        return Err(ConfigError::NoHostname.into());
    }
    if let Some(port) = cmdline.port.or(settings.network.port) {
        client_builder.with_port(port);
    }

    // Add reverse proxy settings to builder.
    if let Some(ref r) = settings.reverse_proxy {
        client_builder.with_reverse_proxy(
            r.port.unwrap_or(s::DEFAULT_REVERSE_PROXY_PORT),
            r.address,
            r.header.clone(),
            r.with_tls_termination
        );
    }

    // Add mempool settings to filter.
    if let Some(mempool_settings) = settings.mempool.clone() {
        client_builder.with_mempool_config(MempoolConfig::from(mempool_settings.clone()));
    }

    // Add TLS configuration, if present.
    // NOTE: Currently we only need to set TLS settings for Wss.
    if settings.network.protocol == s::Protocol::Wss {
        if let Some(tls_settings) = settings.network.tls.clone() {
            client_builder.with_tls_identity(&tls_settings.identity_file, &tls_settings.identity_password);
        }
        else {
            return Err(ConfigError::NoTlsIdentityFile.into())
        }
    }

    // Parse additional seed nodes and add them.
    let seeds = settings.network.seed_nodes.iter()
        .map(|s| s::Seed::try_from(s.clone()))
        .collect::<Result<Vec<Seed>, SeedError>>()?;
    if seeds.iter().any(|s| match s {
        Seed::Peer(uri) => uri.public_key().is_none(),
        _ => false
    }) {
        return Err(ConfigError::MissingPublicKey.into());
    }
    client_builder.with_seeds(seeds);

    client_builder.with_instant_inbound(settings.network.instant_inbound.unwrap_or(false));

    // Setup client future to initialize and connect.
    if network_id.is_albatross() {
        warn!("!!!!");
        warn!("!!!! Albatross node running");
        warn!("!!!!");

        match &settings.validator {
            Some(validator_settings) => {
                let validator_key = {
                    // Load validator key from key store, or create a new one, if key store doesn't exist
                    let key_store_file = validator_settings.key_file.clone()
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
                };
                run_albatross_validator_node(client_builder, settings, validator_config)
            },
            None => {
                info!("No validator");
                info!("Ignoring validator config");
                run_albatross_node(client_builder, settings, ())
            }
        }
    }
    else {
        run_nimiq_node(client_builder, settings, ())?;
    }
}

#[cfg(feature = "rpc-server")]
fn build_rpc_server(rpc_settings: Option<RpcServerSettings>) -> Result<Option<(OtherFuture, Arc<RpcHandler>)>, Error> {
    let rpc_settings = if let Some(s) = rpc_settings {
        s
    } else {
        return Ok(None);
    };

    // Unwrap is safe, since `NetAddress::from_str` only returns variants that can be turned
    // into a IpAddr
    let bind = rpc_settings.bind
        .unwrap_or_else(|| NetAddress::from_str("127.0.0.1").unwrap())
        .into_ip_address().unwrap();
    let port = rpc_settings.port.unwrap_or(s::DEFAULT_RPC_PORT);
    info!("Starting RPC server listening on {}, port {}", bind, port);

    let credentials = match (&rpc_settings.username, &rpc_settings.password) {
        (Some(username), Some(password)) => Some(Credentials::new(&username, &password)),
        (None, None) => None,
        _ => return Err(ConfigError::MissingRpcCredentials.into())
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
    let config = JsonRpcConfig {
        credentials,
        methods: HashSet::from_iter(rpc_settings.methods),
        allowip: (), // TODO
        corsdomain: rpc_settings.corsdomain
    };

    let rpc_handler = Arc::new(RpcHandler::new(config));
    let future = Box::new(rpc_server(bind, port, Arc::clone(&rpc_handler))?);
    Ok(Some((future, rpc_handler)))
}

#[cfg(feature = "rpc-server")]
fn add_generic_rpc_modules<CP>(handler: &Arc<RpcHandler>, consensus: &Arc<Consensus<CP>>) -> Arc<RwLock<UnlockedWalletManager>>
    where CP: ConsensusProtocol
{
    let consensus_handler = ConsensusHandler::new(Arc::clone(&consensus));
    let wallet_handler = WalletHandler::new(consensus.env.clone());
    let network_handler = NetworkHandler::new(&consensus);
    let unlocked_wallets = Arc::clone(&wallet_handler.unlocked_wallets);

    handler.add_module(consensus_handler);
    handler.add_module(network_handler);
    handler.add_module(wallet_handler);

    unlocked_wallets
}

trait ClientConfiguration {
    type Protocol: ConsensusProtocol + 'static;
    type ChainMetrics: AbstractChainMetrics<Self::Protocol> + metrics_server::server::Metrics + 'static;
    type BlockProducer: BlockProducer<Self::Protocol> + 'static;
}

struct NimiqConfiguration();
impl ClientConfiguration for NimiqConfiguration {
    type Protocol = NimiqConsensusProtocol;
    type ChainMetrics = NimiqChainMetrics;
    type BlockProducer = DummyBlockProducer;
}

struct AlbatrossConfiguration();
impl ClientConfiguration for AlbatrossConfiguration {
    type Protocol = AlbatrossConsensusProtocol;
    type ChainMetrics = AlbatrossChainMetrics;
    type BlockProducer = DummyBlockProducer;
}

struct AlbatrossValidatorConfiguration();
impl ClientConfiguration for AlbatrossValidatorConfiguration {
    type Protocol = AlbatrossConsensusProtocol;
    type ChainMetrics = AlbatrossChainMetrics;
    type BlockProducer = AlbatrossBlockProducer;
}
