#[macro_use]
extern crate log;
extern crate nimiq_lib as nimiq;

use std::convert::TryFrom;
use std::time::Duration;

use futures::{future, FutureExt, StreamExt, TryFutureExt};
use tokio::runtime::Runtime;

use nimiq::extras::deadlock::initialize_deadlock_detection;
use nimiq::extras::logging::{initialize_logging, log_error_cause_chain};
use nimiq::extras::panic::initialize_panic_reporting;
use nimiq::prelude::*;

fn main_inner() -> Result<(), Error> {
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

    // We need to instantiate the client within the tokio runtime context, so we use a lazy future.
    let init_future = future::lazy(move |_| {
        // Create client from config.
        info!("Initializing client");
        let client: Client = Client::try_from(config)?;
        client.initialize()?;

        // TODO Initialize RpcServer and MetricsServer or do this in the Client.

        // Initialize network stack and connect.
        info!("Connecting to network");
        client.connect()?;

        Ok(client)
    });

    // Create the "monitor" future which never completes to keep the client alive.
    // This closure is executed after the client has been initialized.
    // TODO Get rid of this. Make the Client a future instead.
    let create_monitor_future = move |client: Client| {
        let mut statistics_interval = config_file.log.statistics;
        let mut show_statistics = true;
        if statistics_interval == 0 {
            statistics_interval = 10;
            show_statistics = false;
        }

        // Run this periodically and optionally show some info.
        tokio::time::interval(Duration::from_secs(statistics_interval))
            .for_each(move |_| {
                if show_statistics {
                    let peer_count = client.network().connections.peer_count();
                    let head = client.blockchain().head().clone();
                    info!(
                        "Head: #{} - {}, Peers: {}",
                        head.block_number(),
                        head.hash(),
                        peer_count
                    );
                }
                future::ready(())
            })
            .map(|_| Ok(()))
    };

    // Start the tokio runtime.
    let mut runtime = Runtime::new()?;
    runtime.block_on(init_future.and_then(create_monitor_future))
}

fn main() {
    if let Err(e) = main_inner() {
        log_error_cause_chain(&e);
    }
}
