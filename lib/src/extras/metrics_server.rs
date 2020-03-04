use metrics_server::MetricsServer;
use metrics_server::server;
use metrics_server::error::Error;
use metrics_server::AbstractChainMetrics;
use consensus::ConsensusProtocol;

use crate::config::config::MetricsServerConfig;
use crate::client::Client;
use crate::extras::block_producer::BlockProducerFactory;
use crate::config::consts::default_bind;


pub fn initialize_metrics_server<P: ConsensusProtocol + BlockProducerFactory, CM: AbstractChainMetrics<P> + server::Metrics + 'static>(client: &Client<P>, config: MetricsServerConfig, pkcs12_key_file: &str, pkcs12_passphrase: &str) -> Result<MetricsServer, Error> {

    let ip = config.bind_to.unwrap_or_else(default_bind);
    info!("Initializing metrics server: {}:{}", ip, config.port);

    let (username, password) = if let Some(credentials) = config.credentials {
        (Some(credentials.username), Some(credentials.password))
    } else {
        warn!("No password set for metrics server!");
        (None, None)
    };

     Ok(MetricsServer::new::<P, CM>(
        ip,
        config.port,
        username,
        password,
        pkcs12_key_file,
        pkcs12_passphrase,
        client.consensus()
    )?)
}
