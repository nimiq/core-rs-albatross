use std::sync::Arc;
use std::time::Duration;

use log::info;

use nimiq::prover::prover_main;
pub use nimiq::{
    client::{Client, Consensus},
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::ConfigFile,
    error::Error,
    extras::{
        deadlock::initialize_deadlock_detection,
        logging::{initialize_logging, log_error_cause_chain},
        metrics_server::NimiqTaskMonitor,
        panic::initialize_panic_reporting,
        signal_handling::initialize_signal_handler,
    },
};

async fn main_inner() -> Result<(), Error> {
    // Initialize deadlock detection
    initialize_deadlock_detection();

    // Parse command line.
    let command_line = CommandLine::parse();
    log::trace!("Command line: {:#?}", command_line);

    // Early return in case of a proving process.
    if command_line.prove {
        // Initialize logging with config values.
        initialize_logging(Some(&command_line), None)?;

        // Initialize panic hook.
        initialize_panic_reporting();

        // Initialize signal handler
        initialize_signal_handler();

        return Ok(prover_main().await?);
    }

    // Parse config file - this will obey the `--config` command line option.
    let config_file = ConfigFile::find(Some(&command_line))?;
    log::trace!("Config file: {:#?}", config_file);

    // Initialize logging with config values.
    initialize_logging(Some(&command_line), Some(&config_file.log))?;

    // Initialize panic hook.
    initialize_panic_reporting();

    // Initialize signal handler
    initialize_signal_handler();

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
    let metrics_config = config.metrics_server.clone();
    let metrics_enabled = metrics_config.is_some();

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

    // Vector for task monitors (Tokio task metrics)
    let mut nimiq_task_metric = vec![];

    // Start consensus.
    let consensus = client.take_consensus().unwrap();

    log::info!("Spawning consensus");
    if metrics_enabled {
        let con_metrics_monitor = tokio_metrics::TaskMonitor::new();
        let instr_con = con_metrics_monitor.instrument(consensus);
        tokio::spawn(instr_con);
        nimiq_task_metric.push(NimiqTaskMonitor {
            name: "consensus".to_string(),
            monitor: con_metrics_monitor,
        });
    } else {
        tokio::spawn(consensus);
    }
    let consensus = client.consensus_proxy();
    let mempool = client.mempool();

    let zkp_component = client.take_zkp_component().unwrap();
    tokio::spawn(zkp_component); //ITODO get metrics on this? ask JD

    // Start validator
    let val_metric_monitor = tokio_metrics::TaskMonitor::new();
    if let Some(validator) = client.take_validator() {
        log::info!("Spawning validator");
        if metrics_enabled {
            let mp_metrics_monitor = validator.get_mempool_monitor();
            let inst_validator = val_metric_monitor.instrument(validator);
            tokio::spawn(inst_validator);
            nimiq_task_metric.push(NimiqTaskMonitor {
                name: "mempool".to_string(),
                monitor: mp_metrics_monitor,
            });
            nimiq_task_metric.push(NimiqTaskMonitor {
                name: "validator".to_string(),
                monitor: val_metric_monitor,
            });
        } else {
            log::info!("Spawning validator");
            tokio::spawn(validator);
        }
    }

    // Start metrics server
    if let Some(metrics_config) = metrics_config {
        nimiq::extras::metrics_server::start_metrics_server(
            metrics_config.addr,
            Arc::clone(&consensus.blockchain),
            mempool,
            client.consensus_proxy(),
            client.network(),
            &nimiq_task_metric,
        );
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

                    info!(
                        block_number = head.block_number(),
                        num_peers = network_info.num_peers(),
                        status = consensus.is_established(),
                        "Consensus status: {:?} - Head: #{}- {}",
                        consensus.is_established(),
                        head.block_number(),
                        head.hash(),
                    )
                }
                Err(err) => {
                    log::error!("Error retrieving NetworkInfo: {:?}", err);
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
