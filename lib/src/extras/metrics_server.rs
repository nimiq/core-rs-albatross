use nimiq_metrics_server::{error::Error, MetricsServer};

use crate::{
    client::Client,
    config::{config::MetricsServerConfig, consts::default_bind},
};

#[allow(unused_variables)]
pub fn initialize_metrics_server(client: &Client, config: MetricsServerConfig, pkcs12_key_file: &str, pkcs12_passphrase: &str) -> Result<MetricsServer, Error> {
    let ip = config.bind_to.unwrap_or_else(default_bind);
    log::info!("Initializing metrics server: {}:{}", ip, config.port);

    let (username, password) = if let Some(credentials) = config.credentials {
        (Some(credentials.username), Some(credentials.password))
    } else {
        log::warn!("No password set for metrics server!");
        (None, None)
    };

    /*Ok(MetricsServer::new::<AlbatrossChainMetrics>(
        ip,
        config.port,
        username,
        password,
        pkcs12_key_file,
        pkcs12_passphrase,
        client.consensus(),
    )?)*/
    todo!()
}
