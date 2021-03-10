use std::time::Duration;

use futures::StreamExt;
pub use nimiq::{
    client::{Client, Consensus},
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::ConfigFile,
    error::Error,
    extras::{
        deadlock::initialize_deadlock_detection,
        logging::{initialize_logging, log_error_cause_chain},
        panic::initialize_panic_reporting,
    },
};

async fn main_inner() -> Result<(), Error> {
    // Initialize deadlock detection
    initialize_deadlock_detection();

    // Parse command line.
    let command_line = CommandLine::from_args();
    log::trace!("Command line: {:#?}", command_line);

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    log::trace!("Config file: {:#?}", config_file);

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
    log::debug!("Final configuration: {:#?}", config);

    // Clone config for RPC and metrics server
    let rpc_config = config.rpc_server.clone();
    let _metrics_config = config.metrics_server.clone();

    // Create client from config.
    log::info!("Initializing client");
    let mut client: Client = Client::from_config(config).await?;
    log::info!("Client initialized");

    // Initialize RPC server
    if let Some(rpc_config) = rpc_config {
        use nimiq::extras::rpc_server::initialize_rpc_server;
        let rpc_server = initialize_rpc_server(&client, rpc_config, client.wallet_store())
            .expect("Failed to initialize RPC server");
        tokio::spawn(async move { rpc_server.run().await });
    }

    // Initialize metrics server
    /*
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
            let metrics_server =
                initialize_metrics_server(&client, metrics_config, pkcs12_key_file, &pkcs12_passphrase).expect("Failed to initialize metrics server");
            //tokio::spawn(metrics_server.into_future());
        } else {
            log::error!("Cannot provide metrics when running without a certificate");
        }
    }
    */

    // Start consensus.
    let mut consensus = client.consensus().unwrap();

    log::info!("Spawning consensus");
    tokio::spawn(async move { consensus.for_each(|_| async {}).await });
    let consensus = client.consensus_proxy();

    // Start validator
    if let Some(validator) = client.validator() {
        log::info!("Spawning validator");
        tokio::spawn(validator);
    } else {
        todo!("Must use validator");
    }

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
            match client.network().network_info().await {
                Ok(network_info) => {
                    let head = client.blockchain_head().clone();

                    log::info!(
                        "Consensus established: {:?} - Head: #{} - {}, Peers: {}",
                        consensus.is_established(),
                        head.block_number(),
                        head.hash(),
                        network_info.num_peers()
                    );
                }
                Err(err) => {
                    log::error!("Error retrieving NEtworkInfo: {:?}", err);
                }
            };
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = main_inner().await {
        log_error_cause_chain(&e);
    }
}
