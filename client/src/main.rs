#[macro_use]
extern crate log;
extern crate nimiq_lib as nimiq;

use std::convert::TryFrom;
use std::time::Duration;

use nimiq::extras::deadlock::initialize_deadlock_detection;
use nimiq::extras::logging::{initialize_logging, log_error_cause_chain};
use nimiq::extras::panic::initialize_panic_reporting;
use nimiq::prelude::*;

async fn main_inner() -> Result<(), Error> {
    // Initialize deadlock detection
    initialize_deadlock_detection();

    // Parse command line.
    let command_line = CommandLine::from_args();
    trace!("Command line: {:#?}", command_line);

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    trace!("Config file: {:#?}", config_file);

    // Initialize logging with config values.
    initialize_logging(Some(&command_line), Some(&config_file.log))?;

    // Initialize panic hook.
    initialize_panic_reporting();

    // Create config builder and apply command line and config file.
    // You usually want the command line to override config settings, so the order is important.
    let mut builder = ClientConfig::builder();
    builder.config_file(&config_file)?;
    builder.command_line(&command_line)?;

    // Finalize config.
    let config = builder.build()?;
    debug!("Final configuration: {:#?}", config);

    // Clone config for RPC and metrics server
    let rpc_config = config.rpc_server.clone();
    let metrics_config = config.metrics_server.clone();
    let protocol_config = config.protocol.clone();

    // Create client from config.
    info!("Initializing client");
    let client: Client = Client::try_from(config)?;
    client.initialize()?;

    // Initialize RPC server
    if let Some(rpc_config) = rpc_config {
        use nimiq::extras::rpc_server::initialize_rpc_server;
        let rpc_server = initialize_rpc_server(&client, rpc_config).expect("Failed to initialize RPC server");
        tokio::spawn(async move { rpc_server.run().await });
    }

    // Initialize metrics server
    if let Some(metrics_config) = metrics_config {
        use nimiq::config::config::ProtocolConfig;
        use nimiq::extras::metrics_server::initialize_metrics_server;
        if let ProtocolConfig::Wss {
            pkcs12_key_file,
            pkcs12_passphrase,
            ..
        } = protocol_config
        {
            let pkcs12_key_file = pkcs12_key_file
                .to_str()
                .unwrap_or_else(|| panic!("Failed to convert path to PKCS#12 key file to string: {}", pkcs12_key_file.display()));

            // FIXME: Spawn `metrics_server` (which is a IntoFuture)
            let _metrics_server =
                initialize_metrics_server(&client, metrics_config, pkcs12_key_file, &pkcs12_passphrase).expect("Failed to initialize metrics server");
            //tokio::spawn(metrics_server.into_future());
        } else {
            error!("Cannot provide metrics when running without a certificate");
        }
    }

    // Initialize network stack and connect.
    info!("Connecting to network");
    client.connect()?;

    // Create the "monitor" future which never completes to keep the client alive.
    // This closure is executed after the client has been initialized.
    // TODO Get rid of this. Make the Client a future/stream instead.
    let mut statistics_interval = config_file.log.statistics;
    let mut show_statistics = true;
    if statistics_interval == 0 {
        statistics_interval = 10;
        show_statistics = false;
    }

    // Run periodically
    let mut interval = tokio::time::interval(Duration::from_secs(statistics_interval));
    loop {
        interval.tick().await;

        if show_statistics {
            let peer_count = client.network().connections.peer_count();
            let head = client.blockchain().head().clone();
            info!("Head: #{} - {}, Peers: {}", head.block_number(), head.hash(), peer_count);
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = main_inner().await {
        log_error_cause_chain(&e);
    }
}
