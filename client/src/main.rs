#[macro_use]
extern crate log;

extern crate nimiq_lib as nimiq;

use std::convert::TryFrom;
use std::time::Duration;

use futures::StreamExt;
use nimiq::prelude::*;
use nimiq::config::config::ProtocolConfig;
use nimiq::extras::logging::initialize_logging;
use nimiq::extras::deadlock::initialize_deadlock_detection;
use nimiq::extras::panic::initialize_panic_reporting;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize deadlock detection
    initialize_deadlock_detection();

    // Parse command line.
    let command_line = CommandLine::from_args();
    trace!("Command line: {:#?}", command_line);

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    trace!("Config file: {:#?}", config_file);

    // Initialize logging with config values
    initialize_logging(Some(&command_line), Some(&config_file.log))?;

    // Initialize panic hook
    initialize_panic_reporting();

    // Create config builder and apply command line and config file
    // You usually want the command line to override config settings, so the order is important
    let mut builder = ClientConfig::builder();
    builder.config_file(&config_file)?;
    builder.command_line(&command_line)?;

    // finalize config
    let config = builder.build()?;
    debug!("Final configuration: {:#?}", config);

    // We need to instantiate the client when the tokio runtime is already alive, so we use
    // a lazy future for it.
    // Clone those now, because we pass ownership of config to Client
    let protocol_config = config.protocol.clone();
    let rpc_config = config.rpc_server.clone();
    let metrics_config = config.metrics_server.clone();

    // Create client from config
    info!("Initializing client");
    let client: Client = Client::try_from(config)?;
    client.initialize().await?;

    // TODO Initialize RPC servers

    // Initialize RPC server
    if let Some(rpc_config) = rpc_config {
        use nimiq::extras::rpc_server::initialize_rpc_server;
        let rpc_server = initialize_rpc_server(&client, rpc_config)
            .expect("Failed to initialize RPC server");
        tokio::spawn(rpc_server.future);
    }

    // Initialize metrics server
    if let Some(mut metrics_config) = metrics_config {
        // FIXME: Use network TLS settings here
        if metrics_config.tls_credentials.is_none() {
            if let ProtocolConfig::Wss { tls_credentials, .. } = protocol_config {
                metrics_config.tls_credentials = Some(tls_credentials);
            }
        }
        tokio::spawn(client.clone().metrics_server(metrics_config));
    }

    // Initialize network stack and connect
    info!("Connecting to network");

    client.connect()?;

    // The Nimiq client is now running and we can access it trough the `client` object.

    // Periodically show some info
    let mut statistics_interval = config_file.log.statistics;
    let mut show_statistics = true;
    if statistics_interval == 0 {
        statistics_interval = 10;
        show_statistics = false;
    }

    let mut interval = tokio::time::interval(Duration::from_secs(statistics_interval));
    while let Some(_) = interval.next().await {
        if show_statistics {
            let peer_count = client.network().connections.peer_count();
            let head = client.blockchain().head().clone();
            info!("Head: #{} - {}, Peers: {}", head.block_number(), head.hash(), peer_count);
        }
    }

    Ok(())
}
