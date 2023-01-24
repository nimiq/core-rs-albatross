use futures::StreamExt;
use gloo_timers::future::IntervalStream;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use log::level_filters::LevelFilter;

pub use nimiq::{
    client::{Client, Consensus},
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::{ConfigFile, LogSettings, SyncMode},
    error::Error,
    extras::{panic::initialize_panic_reporting, web_logging::initialize_web_logging},
};

async fn light_client() {
    let log_settings = LogSettings {
        level: Some(LevelFilter::DEBUG),
        ..Default::default()
    };

    // Initialize logging with config values.
    initialize_web_logging(Some(&log_settings)).expect("Web logging initialization failed");

    // Initialize panic hook.
    initialize_panic_reporting();

    // Create config builder.
    let builder = ClientConfig::builder();

    // Finalize config.
    let config = builder.build().expect("Build configuration failed");
    log::debug!("Final configuration: {:#?}", config);

    // Create client from config.
    log::info!("Initializing light client");
    let mut client: Client = Client::from_config(
        config,
        Box::new(|fut| {
            spawn_local(fut);
        }),
    )
    .await
    .expect("Build client failed");
    log::info!("Web client initialized");

    // Start consensus.
    let consensus = client.take_consensus().unwrap();

    log::info!("Spawning consensus");
    spawn_local(consensus);

    let consensus = client.consensus_proxy();

    let zkp_component = client.take_zkp_component().unwrap();
    spawn_local(zkp_component);

    // Create the "monitor" future which never completes to keep the client alive.
    // This closure is executed after the client has been initialized.
    // TODO Get rid of this. Make the Client a future/stream instead.
    let mut statistics_interval = LogSettings::default_statistics_interval();
    let mut show_statistics = true;
    if statistics_interval == 0 {
        statistics_interval = 10;
        show_statistics = false;
    }

    // Run periodically
    let interval = IntervalStream::new(statistics_interval as u32 * 1000);

    interval
        .for_each(|_| async {
            if show_statistics {
                match client.network().network_info().await {
                    Ok(network_info) => {
                        let head = client.blockchain_head();

                        log::info!(
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
        })
        .await;
}

#[wasm_bindgen]
pub fn nimiq_web_client() {
    spawn_local(light_client());
}
